//! Syntax highlighting: what each part of a line is.

use rechenkern::Calculator;

/// "text:kind" for each part of the line.
fn parts(calc: &Calculator, line: &str) -> String {
    calc.highlight(line)
        .iter()
        .map(|h| format!("{}:{}", &line[h.range.clone()], h.kind.name()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn check(cases: &[(&str, &str)]) {
    let calc = Calculator::new();
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(line, expected)| {
            let got = parts(&calc, line);
            (got != *expected).then(|| format!("{line:30} expected {expected}\n{:30}      got {got}", ""))
        })
        .collect();
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

#[test]
fn values_and_units() {
    check(&[
        ("500 km to mi", "500:number km:unit to:converter mi:unit"),
        ("sqrt(16) + 2 pi", "sqrt:function (:parenthesis 16:number ):parenthesis +:operator 2:number pi:constant"),
        ("20% of 150", "20:number %:operator of:connector 150:number"),
        ("5 in", "5:number in:unit"),
        ("5 in to cm", "5:number in:unit to:converter cm:unit"),
        ("5 ft 3 in", "5:number ft:unit 3:number in:unit"),
        ("255 in hex", "255:number in:converter hex:format"),
        ("10 usd in eur, jpy", "10:number usd:unit in:converter eur, jpy:unit"),
        ("round 3.14159 to 2 dp", "round:function 3.14159:number to:converter 2 dp:format"),
        ("1 cup of flour in grams", "1:number cup:unit of:connector flour:food in:converter grams:unit"),
        (
            "if 5 > 3 then 1 else 2",
            "if:keyword 5:number >:operator 3:number then:keyword 1:number else:keyword 2:number",
        ),
    ]);
}

#[test]
fn dates_and_places() {
    check(&[
        ("time in Paris", "time:date in:converter Paris:city"),
        ("3pm Berlin in Tokyo", "3pm:date Berlin:city in:converter Tokyo:city"),
        ("PST to EST", "PST:time_zone to:converter EST:time_zone"),
        ("5 May 2027 + 3 days", "5 May 2027:date +:operator 3:number days:unit"),
        ("days until christmas", "days:unit until:connector christmas:date"),
        ("sunrise in Tokyo", "sunrise:function in:converter Tokyo:city"),
        ("time in Texas", "time:date in:converter Texas:region"),
        ("lax time", "lax:airport time:date"),
    ]);
}

#[test]
fn notes() {
    check(&[
        ("$20 for lunch + $15", "$:unit 20:number for:comment lunch:comment +:operator $:unit 15:number"),
        ("Rent: $1,500", "Rent::label $:unit 1,500:number"),
        ("# Budget", "# Budget:heading"),
        ("$999 (for iPhone 16)", "$:unit 999:number (:comment for:comment iPhone:comment 16:comment ):comment"),
        (
            "Boeing \"747\" costs $5 // note",
            "Boeing:comment \"747\":comment costs:comment $:unit 5:number // note:comment",
        ),
        ("1. 5 + 5", "1.:list_marker 5:number +:operator 5:number"),
        ("coffee #food $5", "coffee:comment #food:tag $:unit 5:number"),
        ("see https://example.com for $5", "see:comment https://example.com:comment for:comment $:unit 5:number"),
        ("monthly rent = $1500", "monthly rent:variable =:operator $:unit 1500:number"),
        ("line 2 + prev", "line 2:reference +:operator prev:reference"),
    ]);
}

#[test]
fn known_variables() {
    let mut calc = Calculator::new();
    calc.calculate("rent = 5").unwrap();
    assert_eq!(parts(&calc, "rent * 2"), "rent:variable *:operator 2:number");
}
