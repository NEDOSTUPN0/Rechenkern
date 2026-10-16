//! Sheets as plain text, JSON and HTML, and archives to save and load them, as
//! SoulverCore's `LineCollectionExporter`, `LineCollectionHTMLExporter` and
//! `LineCollectionArchiver`.

use std::fmt::Write;

use super::{Alignment, Behaviour, Color, Sheet, Statistic};
use crate::calculator::Calculator;
use crate::json::Json;
use crate::number::Number;
use crate::parser::highlight::{Highlight, TokenKind};
use crate::value::Value;
use crate::zones::PlaceKind;

/// How a sheet is written as plain text.
#[derive(Clone, Debug)]
pub struct TextOptions {
    /// Between a line and its answer.
    pub separator: String,
    /// Answers with every digit, no separators and currency codes.
    pub unformatted: bool,
    /// `prev` and `line 2` are written as the answers they stand for.
    pub substitute_references: bool,
    pub include_answers: bool,
    pub style: TextStyle,
    pub line_numbers: bool,
    /// A last line with a statistic of the sheet: "Total: $1,900.00".
    pub statistic: Option<Statistic>,
    pub hide_comments: bool,
    /// Headings without their `#`.
    pub hide_heading_indicator: bool,
}

impl Default for TextOptions {
    fn default() -> TextOptions {
        TextOptions {
            separator: " = ".into(),
            unformatted: false,
            substitute_references: false,
            include_answers: true,
            style: TextStyle::Regular,
            line_numbers: false,
            statistic: None,
            hide_comments: false,
            hide_heading_indicator: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextStyle {
    /// "rent = $1,500 = $1,500.00"
    #[default]
    Regular,
    /// Answers lined up in a column after the longest line.
    Pretty,
}

/// How a sheet is written as HTML, as SoulverCore's `HTMLExportOptions`.
#[derive(Clone, Debug)]
pub struct HtmlOptions {
    pub line_numbers: bool,
    /// A heading over the sheet; without one the first heading line is the title.
    pub heading: Option<String>,
    /// Numbers, units, functions and the rest in their colors.
    pub color_text: bool,
    /// A total row at the end, with this label.
    pub total_label: Option<String>,
    pub font_family: String,
    pub font_size: f64,
    pub heading_size: f64,
    /// A date under the sheet: "Modified 23 Sep 2026".
    pub modified: Option<String>,
    pub footer: bool,
    pub hide_comments: bool,
    pub hide_heading_indicator: bool,
    /// Without line breaks and indentation.
    pub minify: bool,
    pub dark: bool,
    pub answer_alignment: Alignment,
    /// Answers below the first number or above the second take the small and large number colors.
    pub number_thresholds: Option<(Number, Number)>,
}

impl Default for HtmlOptions {
    fn default() -> HtmlOptions {
        HtmlOptions {
            line_numbers: false,
            heading: None,
            color_text: true,
            total_label: None,
            font_family: "-apple-system, system-ui, sans-serif".into(),
            font_size: 14.0,
            heading_size: 14.0,
            modified: None,
            footer: true,
            hide_comments: false,
            hide_heading_indicator: false,
            minify: false,
            dark: false,
            answer_alignment: Alignment::Left,
            number_thresholds: None,
        }
    }
}

/// Colors of an HTML sheet, as CSS colors: "#0b66e4".
#[derive(Clone, Debug)]
pub struct HtmlColors {
    /// Text of each kind of token; kinds not listed keep the text color.
    pub tokens: Vec<(TokenKind, String)>,
    /// Ink and pill colors of tags: `("food", ("#fff", "#e67e22"))`.
    pub tags: Vec<(String, (String, String))>,
    pub text: String,
    pub background: String,
    pub answer_background: String,
    pub answer_line: String,
    pub answer_text: String,
    pub heading: String,
    pub link: String,
    pub small_numbers: Option<String>,
    pub large_numbers: Option<String>,
}

impl HtmlColors {
    pub fn light() -> HtmlColors {
        HtmlColors {
            tokens: token_colors(false),
            tags: Vec::new(),
            text: "#1d1d1f".into(),
            background: "#ffffff".into(),
            answer_background: "#fbfbfb".into(),
            answer_line: "#e2e2e2".into(),
            answer_text: "#000000".into(),
            heading: "#000000".into(),
            link: "#007aff".into(),
            small_numbers: Some("#c0392b".into()),
            large_numbers: Some("#1e8449".into()),
        }
    }

    pub fn dark() -> HtmlColors {
        HtmlColors {
            tokens: token_colors(true),
            tags: Vec::new(),
            text: "#e5e5e7".into(),
            background: "#1e1e1e".into(),
            answer_background: "#252526".into(),
            answer_line: "#3a3a3c".into(),
            answer_text: "#ffffff".into(),
            heading: "#ffffff".into(),
            link: "#0a84ff".into(),
            small_numbers: Some("#ff6b5e".into()),
            large_numbers: Some("#5fd38d".into()),
        }
    }
}

impl Default for HtmlColors {
    fn default() -> HtmlColors {
        HtmlColors::light()
    }
}

fn token_colors(dark: bool) -> Vec<(TokenKind, String)> {
    let pick = |light: &str, night: &str| if dark { night.to_string() } else { light.to_string() };
    vec![
        (TokenKind::Number, pick("#0b66e4", "#6cb4ff")),
        (TokenKind::Unit, pick("#008080", "#4fd1c5")),
        (TokenKind::Function, pick("#8e44ad", "#c39bd3")),
        (TokenKind::Variable, pick("#d35400", "#f5a65b")),
        (TokenKind::Reference, pick("#d35400", "#f5a65b")),
        (TokenKind::Date, pick("#1e8449", "#7dcea0")),
        (TokenKind::Place(PlaceKind::City), pick("#a04000", "#e59866")),
        (TokenKind::Converter, pick("#c2185b", "#f48fb1")),
        (TokenKind::Comment, pick("#8e8e93", "#8e8e93")),
        (TokenKind::Label, pick("#8e8e93", "#a1a1a6")),
        (TokenKind::Tag, pick("#e67e22", "#f0b27a")),
    ]
}

impl Sheet {
    /// The sheet as plain text, as SoulverCore's plain text export.
    pub fn to_text(&self, options: &TextOptions) -> String {
        let rows: Vec<(String, Option<String>)> = (0..self.len())
            .map(|i| {
                let expression = self.expression(
                    i,
                    options.hide_comments,
                    options.substitute_references,
                    options.hide_heading_indicator,
                );
                let answer = options.include_answers.then(|| self.answer_text(i, options.unformatted)).flatten();
                let number = if options.line_numbers { format!("{}. ", i + 1) } else { String::new() };
                (format!("{number}{expression}"), answer)
            })
            .collect();
        let width = rows.iter().filter(|(_, a)| a.is_some()).map(|(e, _)| e.chars().count()).max().unwrap_or(0);
        let mut out = Vec::new();
        for (expression, answer) in rows {
            out.push(match (answer, options.style) {
                (None, _) => expression,
                (Some(answer), TextStyle::Regular) => format!("{expression}{}{answer}", options.separator),
                (Some(answer), TextStyle::Pretty) => {
                    let pad = " ".repeat(width - expression.chars().count());
                    format!("{expression}{pad}{}{answer}", options.separator)
                }
            });
        }
        if let Some(statistic) = options.statistic
            && let Some(Ok(answer)) = self.statistic(statistic, None)
        {
            out.push(format!("{}: {answer}", statistic_name(statistic)));
        }
        out.join("\n")
    }

    /// The sheet as JSON: one object per line with its text, answer and marks.
    pub fn to_json(&self) -> String {
        let lines = (0..self.len())
            .map(|i| {
                let line = &self.lines[i];
                let (answer, error) = match line.answer() {
                    Ok(answer) => (Json::from(answer.map(|a| a.text().to_string())), Json::Null),
                    Err(e) => (Json::Null, Json::from(e.to_string())),
                };
                Json::Object(vec![
                    ("line".into(), Json::Number((i + 1) as f64)),
                    ("text".into(), line.text().into()),
                    ("answer".into(), answer),
                    ("error".into(), error),
                    ("raw".into(), self.answer_text(i, true).into()),
                    ("variable".into(), line.variable().into()),
                    ("running".into(), line.running().map(|a| a.text().to_string()).into()),
                    ("behaviour".into(), behaviour_name(line.behaviour).into()),
                ])
            })
            .collect();
        Json::Array(lines).to_string()
    }

    /// The sheet as an HTML page, as SoulverCore's HTML export.
    pub fn to_html(&self, options: &HtmlOptions, colors: &HtmlColors) -> String {
        self.html(options, colors, false)
    }

    /// The sheet as HTML for an email: styles inline, no style sheet.
    pub fn to_email_html(&self, options: &HtmlOptions, colors: &HtmlColors) -> String {
        self.html(options, colors, true)
    }

    /// Text that `Sheet::unarchive` reads back: every line with its marks, as JSON.
    /// Answers are worked out again when it is read.
    pub fn archive(&self) -> String {
        let lines = self
            .lines
            .iter()
            .map(|line| {
                let highlights = line.highlights.iter().map(|h| Json::from(h.map(color_name))).collect::<Vec<_>>();
                Json::Object(vec![
                    ("text".into(), line.text().into()),
                    ("behaviour".into(), behaviour_name(line.behaviour).into()),
                    ("highlights".into(), Json::Array(highlights)),
                    ("alignment".into(), alignment_name(line.alignment).into()),
                    ("attribution".into(), line.attribution.clone().into()),
                ])
            })
            .collect();
        Json::Object(vec![("version".into(), Json::Number(1.0)), ("lines".into(), Json::Array(lines))]).to_string()
    }

    /// A sheet from `archive`, worked out with this calculator.
    pub fn unarchive(calc: Calculator, archive: &str) -> crate::Result<Sheet> {
        let bad = || crate::Error::new("not a sheet archive");
        let json = Json::parse(archive).ok_or_else(bad)?;
        if json.get("version").and_then(Json::as_f64).is_none_or(|v| v > 1.0) {
            return Err(crate::Error::new("a sheet archive of a newer version"));
        }
        let lines = json.get("lines").and_then(Json::as_array).ok_or_else(bad)?;
        let mut sheet = Sheet::new(calc);
        let texts: Vec<&str> = lines.iter().map(|l| l.get("text").and_then(Json::as_str).unwrap_or("")).collect();
        sheet.lines.clear();
        for (line, text) in lines.iter().zip(texts) {
            let id = sheet.new_id();
            let mut new = super::SheetLine::new(id, text);
            let name = |key: &str| line.get(key).and_then(Json::as_str);
            new.behaviour = name("behaviour").and_then(behaviour_named).unwrap_or_default();
            new.alignment = name("alignment").and_then(alignment_named).unwrap_or_default();
            new.attribution = name("attribution").map(String::from);
            if let Some(highlights) = line.get("highlights").and_then(Json::as_array) {
                for (slot, color) in new.highlights.iter_mut().zip(highlights) {
                    *slot = color.as_str().and_then(color_named);
                }
            }
            sheet.lines.push(new);
        }
        sheet.recalculate();
        Ok(sheet)
    }

    /// The text of line `i` for an export.
    fn expression(&self, i: usize, hide_comments: bool, substitute: bool, hide_heading: bool) -> String {
        let text = self.lines[i].text();
        if self.lines[i].is_heading() {
            return if hide_heading { text.trim_start_matches('#').trim_start().to_string() } else { text.to_string() };
        }
        if !hide_comments && !substitute {
            return text.to_string();
        }
        let mut out = String::new();
        let mut at = 0;
        for h in self.highlights(i) {
            let part = &text[h.range.clone()];
            out.push_str(&text[at..h.range.start]);
            at = h.range.end;
            match h.kind {
                TokenKind::Comment | TokenKind::Tag if hide_comments => {}
                TokenKind::Reference if substitute => match self.referenced(i, part) {
                    Some(answer) => out.push_str(&answer),
                    None => out.push_str(part),
                },
                _ => out.push_str(part),
            }
        }
        out.push_str(&text[at..]);
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// The answer a reference on line `i` stands for: "line 2" or "prev".
    fn referenced(&self, i: usize, reference: &str) -> Option<String> {
        let lower = reference.to_lowercase();
        let line = match lower.strip_prefix("line").map(str::trim) {
            Some(n) if !n.is_empty() => n.parse::<usize>().ok()?.checked_sub(1)?,
            _ if matches!(lower.as_str(), "prev" | "previous" | "ans" | "answer" | "above") => {
                (0..i).rev().find(|&k| matches!(self.lines[k].answer(), Ok(Some(_))))?
            }
            _ => return None,
        };
        Some(self.lines.get(line)?.answer().ok()??.text().to_string())
    }

    /// The answer of line `i`, as shown or with every digit.
    fn answer_text(&self, i: usize, unformatted: bool) -> Option<String> {
        let answer = self.lines[i].answer().ok()??;
        Some(if unformatted { answer.unformatted_text() } else { answer.text().to_string() })
    }

    fn html(&self, options: &HtmlOptions, colors: &HtmlColors, email: bool) -> String {
        let nl = if options.minify { "" } else { "\n" };
        let mut out = String::new();
        let align = if options.answer_alignment == Alignment::Right { "right" } else { "left" };
        let body_style = format!(
            "font-family:{};font-size:{}px;color:{};background:{};margin:0;padding:16px",
            options.font_family, options.font_size, colors.text, colors.background
        );
        let cell = "padding:2px 8px;vertical-align:top;white-space:pre-wrap";
        let answer_style = format!(
            "{cell};text-align:{align};background:{};color:{};border-left:1px solid {}",
            colors.answer_background, colors.answer_text, colors.answer_line
        );
        let heading_style = format!("font-size:{}px;font-weight:600;color:{}", options.heading_size, colors.heading);
        let title = options.heading.clone().or_else(|| {
            self.lines.iter().find(|l| l.is_heading()).map(|l| l.text().trim_start_matches('#').trim().to_string())
        });
        if !email {
            out.push_str("<!doctype html>");
            out.push_str(nl);
            let _ = write!(
                out,
                "<html><head><meta charset=\"utf-8\"><title>{}</title>",
                escape(title.as_deref().unwrap_or("Sheet"))
            );
            let _ = write!(
                out,
                "<style>body{{{body_style}}}table{{border-collapse:collapse;width:100%}}td{{{cell}}}td.answer{{{answer_style}}}td.n{{color:#8e8e93;text-align:right}}tr.heading td{{{heading_style}}}a{{color:{}}}",
                colors.link
            );
            for (kind, color) in &colors.tokens {
                let _ = write!(out, ".{}{{color:{color}}}", kind.name());
            }
            for (tag, (ink, pill)) in &colors.tags {
                let _ = write!(
                    out,
                    ".tag-{}{{color:{ink};background:{pill};border-radius:4px;padding:0 3px}}",
                    css_name(tag)
                );
            }
            out.push_str("</style></head>");
            out.push_str(nl);
            out.push_str("<body>");
        } else {
            let _ = write!(out, "<div style=\"{body_style}\">");
        }
        out.push_str(nl);
        if let Some(heading) = &options.heading {
            let style = if email { format!(" style=\"{heading_style}\"") } else { String::new() };
            let _ = write!(out, "<h1{style}>{}</h1>{nl}", escape(heading));
        }
        let table_style = if email { " style=\"border-collapse:collapse;width:100%\"" } else { "" };
        let _ = write!(out, "<table{table_style}>{nl}");
        for i in 0..self.len() {
            let line = &self.lines[i];
            let heading = line.is_heading();
            let row_style = if email && heading { format!(" style=\"{heading_style}\"") } else { String::new() };
            let class = if heading { " class=\"heading\"" } else { "" };
            let _ = write!(out, "<tr{}{}>", if email { "" } else { class }, row_style);
            if options.line_numbers {
                let style = if email {
                    format!(" style=\"{cell};color:#8e8e93;text-align:right\"")
                } else {
                    " class=\"n\"".into()
                };
                let _ = write!(out, "<td{style}>{}</td>", i + 1);
            }
            let text_style = if email { format!(" style=\"{cell}\"") } else { String::new() };
            let _ = write!(out, "<td{text_style}>{}</td>", self.html_expression(i, options, colors, email));
            let answer = self.lines[i].answer().ok().flatten();
            let color = answer.and_then(|a| number_color(a.value(), options, colors));
            let answer_attr = match (email, color) {
                (true, Some(c)) => format!(" style=\"{answer_style};color:{c}\""),
                (true, None) => format!(" style=\"{answer_style}\""),
                (false, Some(c)) => format!(" class=\"answer\" style=\"color:{c}\""),
                (false, None) => " class=\"answer\"".into(),
            };
            let _ = write!(out, "<td{answer_attr}>{}</td></tr>{nl}", escape(answer.map_or("", |a| a.text())));
        }
        if let Some(label) = &options.total_label
            && let Some(Ok(total)) = self.statistic(Statistic::Total, None)
        {
            let span = if options.line_numbers { " colspan=\"2\"" } else { "" };
            let style =
                if email { format!(" style=\"{cell};font-weight:600\"") } else { " style=\"font-weight:600\"".into() };
            let answer_attr = if email {
                format!(" style=\"{answer_style};font-weight:600\"")
            } else {
                " class=\"answer\" style=\"font-weight:600\"".into()
            };
            let _ = write!(
                out,
                "<tr><td{span}{style}>{}</td><td{answer_attr}>{}</td></tr>{nl}",
                escape(label),
                escape(total.text())
            );
        }
        out.push_str("</table>");
        out.push_str(nl);
        if let Some(modified) = &options.modified {
            let _ = write!(out, "<p style=\"color:#8e8e93\">Modified {}</p>{nl}", escape(modified));
        }
        if options.footer {
            let _ = write!(out, "<p style=\"color:#8e8e93;font-size:smaller\">Made with Rechenkern</p>{nl}");
        }
        out.push_str(if email { "</div>" } else { "</body></html>" });
        out.push_str(nl);
        out
    }

    /// The text of line `i` as HTML, with its parts in their colors.
    fn html_expression(&self, i: usize, options: &HtmlOptions, colors: &HtmlColors, email: bool) -> String {
        let line = &self.lines[i];
        if line.is_heading() || !options.color_text {
            return escape(&self.expression(i, options.hide_comments, false, options.hide_heading_indicator));
        }
        let text = line.text();
        let mut out = String::new();
        let mut at = 0;
        for Highlight { range, kind } in self.highlights(i) {
            out.push_str(&escape(&text[at..range.start]));
            at = range.end;
            let part = &text[range.clone()];
            if options.hide_comments && matches!(kind, TokenKind::Comment | TokenKind::Tag) {
                continue;
            }
            let tag_colors = colors
                .tags
                .iter()
                .find(|(t, _)| kind == TokenKind::Tag && part.trim_start_matches('#').eq_ignore_ascii_case(t));
            let color = colors.tokens.iter().find(|(k, _)| *k == kind).map(|(_, c)| c.as_str());
            let _ = match (email, tag_colors, color) {
                (true, Some((_, (ink, pill))), _) => {
                    write!(
                        out,
                        "<span style=\"color:{ink};background:{pill};border-radius:4px;padding:0 3px\">{}</span>",
                        escape(part)
                    )
                }
                (false, Some((tag, _)), _) => {
                    write!(out, "<span class=\"tag-{}\">{}</span>", css_name(tag), escape(part))
                }
                (true, None, Some(color)) => write!(out, "<span style=\"color:{color}\">{}</span>", escape(part)),
                (false, None, _) => write!(out, "<span class=\"{}\">{}</span>", kind.name(), escape(part)),
                (true, None, None) => write!(out, "{}", escape(part)),
            };
        }
        out.push_str(&escape(&text[at..]));
        out
    }
}

/// The small or large number color for an answer, by the thresholds.
fn number_color<'a>(value: &Value, options: &HtmlOptions, colors: &'a HtmlColors) -> Option<&'a str> {
    let (small, large) = options.number_thresholds?;
    let n = match value {
        Value::Quantity(q) => q.number,
        Value::Percent(p) => *p,
        _ => return None,
    };
    if n < small {
        colors.small_numbers.as_deref()
    } else if n > large {
        colors.large_numbers.as_deref()
    } else {
        None
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn css_name(tag: &str) -> String {
    tag.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect()
}

fn statistic_name(statistic: Statistic) -> &'static str {
    match statistic {
        Statistic::Total => "Total",
        Statistic::Average => "Average",
        Statistic::Count => "Count",
        Statistic::Median => "Median",
        Statistic::Min => "Min",
        Statistic::Max => "Max",
        Statistic::Lcm => "LCM",
        Statistic::Gcd => "GCD",
        Statistic::StandardDeviation => "Standard deviation",
    }
}

const BEHAVIOURS: &[(Behaviour, &str)] = &[
    (Behaviour::Expression, "expression"),
    (Behaviour::Subtotal, "subtotal"),
    (Behaviour::GrandTotal, "grand_total"),
    (Behaviour::RunningTotal, "running_total"),
    (Behaviour::RunningBudget, "running_budget"),
];

fn behaviour_name(behaviour: Behaviour) -> &'static str {
    BEHAVIOURS.iter().find(|(b, _)| *b == behaviour).map_or("expression", |(_, n)| n)
}

fn behaviour_named(name: &str) -> Option<Behaviour> {
    BEHAVIOURS.iter().find(|(_, n)| *n == name).map(|(b, _)| *b)
}

const COLORS: &[(Color, &str)] = &[
    (Color::Orange, "orange"),
    (Color::Blue, "blue"),
    (Color::Purple, "purple"),
    (Color::Pink, "pink"),
    (Color::Red, "red"),
    (Color::Yellow, "yellow"),
    (Color::Green, "green"),
    (Color::Grey, "grey"),
];

fn color_name(color: Color) -> &'static str {
    COLORS.iter().find(|(c, _)| *c == color).map_or("grey", |(_, n)| n)
}

fn color_named(name: &str) -> Option<Color> {
    COLORS.iter().find(|(_, n)| *n == name).map(|(c, _)| *c)
}

fn alignment_name(alignment: Alignment) -> &'static str {
    match alignment {
        Alignment::Left => "left",
        Alignment::Center => "center",
        Alignment::Right => "right",
    }
}

fn alignment_named(name: &str) -> Option<Alignment> {
    Some(match name {
        "left" => Alignment::Left,
        "center" => Alignment::Center,
        "right" => Alignment::Right,
        _ => return None,
    })
}
