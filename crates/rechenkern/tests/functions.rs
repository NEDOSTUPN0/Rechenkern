//! Functions of the host's own (phrases with placeholders) and functions defined in a sheet.

use rechenkern::{Calculator, Config, CustomFunction, CustomType, Form, Number, Value};

fn answers(calc: &mut Calculator, lines: &[(&str, &str)]) {
    let failures: Vec<String> = lines
        .iter()
        .filter_map(|(line, expected)| {
            let got = match calc.calculate(line) {
                Ok(Some(answer)) => answer.to_string(),
                Ok(None) => "<none>".into(),
                Err(e) => format!("<error: {e}>"),
            };
            (got != *expected).then(|| format!("{line:35} expected {expected:25} got {got}"))
        })
        .collect();
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

/// 15% of an amount.
fn tip(values: &[Value]) -> Result<Value, String> {
    match &values[0] {
        Value::Quantity(q) => {
            let mut q = q.clone();
            q.number = q.number * Number::parse("0.15").unwrap();
            Ok(Value::Quantity(q))
        }
        _ => Err("no amount".into()),
    }
}

fn split(values: &[Value]) -> Result<Value, String> {
    match (&values[0], &values[1]) {
        (Value::Quantity(total), Value::Quantity(ways)) => {
            let mut each = total.clone();
            each.number = total.number / ways.number;
            Ok(Value::Quantity(each))
        }
        _ => Err("expected an amount and a number".into()),
    }
}

#[test]
fn host_phrases() {
    let mut tip = CustomFunction::new("tip on __money", tip);
    tip.aliases.push("__money tip".into());
    let airline =
        CustomType { placeholder: "__airline".into(), words: vec!["QF".into(), "BA".into(), "Qantas".into()] };
    let flight = CustomFunction::new("flight __airline __number", |values| match values {
        [Value::Text(airline), Value::Quantity(n)] => Ok(Value::Text(format!("{airline}{}", n.number))),
        _ => Err("bad flight".into()),
    });
    let config = Config {
        custom_functions: vec![tip, CustomFunction::new("__money split __number ways", split), flight],
        custom_types: vec![airline],
        ..Config::default()
    };
    let mut calc = Calculator::with_config(config);
    answers(
        &mut calc,
        &[
            ("tip on $80", "$12.00"),
            ("$80 tip", "$12.00"),
            ("TIP ON $40 + $40", "$12.00"),
            ("$90 split 3 ways", "$30.00"),
            ("$90 + $30 split 4 ways", "$30.00"),
            ("flight QF 123", "QF123"),
            ("tip on 80", "<error: \"tip on __money\" needs another kind of value than a number>"),
            ("tip for $80", "$80.00"),
        ],
    );
    assert_eq!(calc.calculate("tip on $80").unwrap().unwrap().form(), Form::CustomFunction);
    let values = Calculator::new().function_parameters("$90 split 3 ways", &["__money split __number ways"]).unwrap();
    assert_eq!(values.len(), 2);
    assert!(matches!(&values[1], Value::Quantity(q) if q.number == Number::from_i64(3)));
    assert!(Calculator::new().function_parameters("90 apples", &["__money split __number ways"]).is_none());
}

#[test]
fn functions_in_lines() {
    let mut calc = Calculator::new();
    answers(
        &mut calc,
        &[
            ("area(w, h) = w * h", "<none>"),
            ("area(3, 4)", "12"),
            ("area(2 m, 3 m)", "6 m²"),
            ("area(3)", "<error: area takes 2 values, not 1>"),
            ("tip(amount) = amount * 15%", "<none>"),
            ("tip($80) + area(1, 2)", "$14.00"),
            ("rate = 2", "2"),
            ("scaled(x) = x * rate", "<none>"),
            ("scaled(5)", "10"),
            ("fac(n) = n * fac(n - 1) if n > 1 else 1", "<none>"),
            ("fac(5)", "120"),
            ("loop(n) = loop(n)", "<none>"),
            ("loop(1)", "<error: loop calls itself too deep>"),
            ("sqrt(x) = x", "<error: sqrt is a function already>"),
            ("double(x) = x", "<error: double means something already>"),
            ("km(x) = x", "<error: km means something already>"),
            ("---", "<none>"),
            ("area(3, 4)", "<error: unknown function \"area\">"),
        ],
    );
}

#[test]
fn functions_in_sheets() {
    let mut sheet = rechenkern::Sheet::default();
    sheet.set_text("dbl(x) = x * 2\ndbl(21)\n5 + 5");
    assert_eq!(sheet.lines()[1].answer().unwrap().unwrap().text(), "42");
    assert_eq!(sheet.set_line(0, "dbl(x) = x * 3"), [0, 1]);
    assert_eq!(sheet.lines()[1].answer().unwrap().unwrap().text(), "63");
}
