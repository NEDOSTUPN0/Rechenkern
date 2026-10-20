//! Help for editors: token trees, a line's parts, tidy input, what a word is.

use rechenkern::{Beautify, Calculator, TokenKind};

#[test]
fn token_tree() {
    let calc = Calculator::new();
    let tokens = calc.tokens("sqrt(16) + (2 * 3) km");
    let kinds: Vec<_> = tokens.iter().map(|t| (t.kind, t.text.as_str())).collect();
    assert_eq!(
        kinds,
        [
            (TokenKind::Function, "sqrt(16)"),
            (TokenKind::Operator, "+"),
            (TokenKind::Parenthesis, "(2 * 3)"),
            (TokenKind::Unit, "km"),
        ]
    );
    assert_eq!(tokens[0].children[0].children[0].text, "16");
    assert_eq!(tokens[2].children.len(), 3);
    assert_eq!(calc.token_at("sqrt(16) + (2 * 3) km", 14).unwrap().text, "*");
    assert_eq!(calc.token_at("€5 + 3", 1).unwrap().kind, TokenKind::Number);
    // Open parentheses don't lose tokens.
    assert_eq!(calc.tokens("(2 + 3").len(), 4);
}

#[test]
fn line_parts() {
    let calc = Calculator::new();
    let line = "1. Rent: $1,500 per month #home";
    let parts = calc.components(line);
    assert_eq!(&line[parts.list_marker.unwrap()], "1.");
    assert_eq!(&line[parts.label.unwrap()], "Rent:");
    assert_eq!(parts.tags, ["home"]);
    assert_eq!(&line[parts.body.clone()], "$1,500 per month");
    assert_eq!(&line[parts.tail.clone()], " #home");
    assert_eq!(&line[parts.head.clone()], "1. Rent: ");

    let parts = calc.components("## Budget");
    assert_eq!(parts.heading, Some((2, 3..9)));
    let parts = calc.components("monthly rent = $1500 // since May");
    assert_eq!(parts.variable.as_deref(), Some("monthly rent"));
    assert_eq!(parts.comments.len(), 1);
}

#[test]
fn tidy_input() {
    let calc = Calculator::new();
    let tidy = |line| calc.beautify(line, &Beautify::default());
    assert_eq!(tidy("5*3-2"), "5 × 3 − 2");
    assert_eq!(tidy("12000 + -5"), "12,000 + -5");
    assert_eq!(tidy("$20 for lunch+$15"), "$20 for lunch + $15");
    assert_eq!(tidy("x=10/4"), "x = 10 / 4");
    let plain = Beautify { spaced_operators: false, thousands_separators: false, ..Beautify::default() };
    assert_eq!(calc.beautify("3*4-1", &plain), "3×4−1");
}

#[test]
fn what_a_word_is() {
    let mut calc = Calculator::new();
    assert!(calc.is_operator("×") && calc.is_operator("divided by") && !calc.is_operator("km"));
    assert!(calc.is_function("sqrt") && calc.is_function("Round") && !calc.is_function("tokyo"));
    calc.calculate("area(w, h) = w * h").unwrap();
    assert!(calc.is_function("area"));
    assert!(calc.is_place("Tokyo") && calc.is_place("JFK") && !calc.is_place("banana"));
    assert!(calc.is_unit("km") && calc.is_unit("Miles per hour") && !calc.is_unit("tokyo"));
}

#[test]
fn other_ranges() {
    let calc = Calculator::new();
    let line = "€5 + 😀 3";
    let three = calc.highlight(line).into_iter().last().unwrap();
    assert_eq!(three.char_range(line), 7..8);
    assert_eq!(three.utf16_range(line), 8..9);
}
