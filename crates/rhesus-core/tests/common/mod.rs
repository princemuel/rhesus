#![cfg(test)]

use rhesus_core::{Engine, Session};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Expected<'a> {
    Int(i64),
    Bool(bool),
    Str(&'a str),
    Null,
    Err(&'a str), // Matches substring in runtime/compile error
}

/// Evaluates a Monkey snippet on a specific engine and checks the result
pub(crate) fn assert_eval(engine: Engine, input: &str, expected: Expected<'_>) {
    let mut session = Session::new(engine);
    let actual = session.eval_line(input);

    match (actual, expected) {
        (Ok(val), Expected::Int(n)) => {
            assert_eq!(val, n.to_string(), "[{engine:?}] failed on: {input}");
        }
        (Ok(val), Expected::Bool(b)) => {
            assert_eq!(val, b.to_string(), "[{engine:?}] failed on: {input}");
        }
        (Ok(val), Expected::Str(s)) => {
            assert_eq!(val, s, "[{engine:?}] failed on: {input}");
        }
        (Ok(val), Expected::Null) => {
            assert_eq!(val, "null", "[{engine:?}] failed on: {input}");
        }
        (Err(err), Expected::Err(substr)) => {
            assert!(
                err.contains(substr),
                "[{engine:?}] expected error containing '{substr}', got: '{err}' on: {input}"
            );
        }
        (Ok(val), Expected::Err(substr)) => {
            panic!("[{engine:?}] expected error '{substr}', but got Ok('{val}') on: {input}");
        }
        (Err(err), exp) => {
            panic!("[{engine:?}] expected {exp:?}, but got Err('{err}') on: {input}");
        }
    }
}

/// Runs a table of (input, expected) test cases on the chosen engine
pub(crate) fn run_cases(engine: Engine, cases: &[(&str, Expected<'_>)]) {
    for (input, expected) in cases {
        // Reconstruct Expected cleanly for each case
        match expected {
            Expected::Int(n) => assert_eval(engine, input, Expected::Int(*n)),
            Expected::Bool(b) => assert_eval(engine, input, Expected::Bool(*b)),
            Expected::Str(s) => assert_eval(engine, input, Expected::Str(s)),
            Expected::Null => assert_eval(engine, input, Expected::Null),
            Expected::Err(e) => assert_eval(engine, input, Expected::Err(e)),
        }
    }
}

pub(crate) fn test_integer_arithmetic(engine: Engine) {
    let cases = [
        ("5", Expected::Int(5)),
        ("10", Expected::Int(10)),
        ("-5", Expected::Int(-5)),
        ("-10", Expected::Int(-10)),
        ("5 + 5 + 5 + 5 - 10", Expected::Int(10)),
        ("2 * 2 * 2 * 2 * 2", Expected::Int(32)),
        ("5 * 2 + 10", Expected::Int(20)),
        ("5 + 2 * 10", Expected::Int(25)),
        ("(5 + 10 * 2 + 15 / 3) * 2 + -10", Expected::Int(50)),
    ];
    run_cases(engine, &cases);
}

pub(crate) fn test_boolean_expressions(engine: Engine) {
    let cases = [
        ("true", Expected::Bool(true)),
        ("false", Expected::Bool(false)),
        ("1 < 2", Expected::Bool(true)),
        ("1 > 2", Expected::Bool(false)),
        ("1 == 1", Expected::Bool(true)),
        ("1 != 1", Expected::Bool(false)),
        ("!true", Expected::Bool(false)),
        ("!!true", Expected::Bool(true)),
        ("(1 < 2) == true", Expected::Bool(true)),
    ];
    run_cases(engine, &cases);
}

pub(crate) fn test_conditionals(engine: Engine) {
    let cases = [
        ("if (true) { 10 }", Expected::Int(10)),
        ("if (false) { 10 }", Expected::Null),
        ("if (1 < 2) { 10 } else { 20 }", Expected::Int(10)),
        ("if (1 > 2) { 10 } else { 20 }", Expected::Int(20)),
    ];
    run_cases(engine, &cases);
}

pub(crate) fn test_closures_and_functions(engine: Engine) {
    let cases = [
        ("let identity = fn(x) { x; }; identity(5);", Expected::Int(5)),
        ("let add = fn(x, y) { x + y; }; add(5 + 5, add(5, 5));", Expected::Int(20)),
        (
            "let newAdder = fn(x) { fn(y) { x + y }; };
             let addTwo = newAdder(2);
             addTwo(3);",
            Expected::Int(5),
        ),
    ];
    run_cases(engine, &cases);
}

pub(crate) fn test_error_handling(engine: Engine) {
    let cases = [
        ("5 + true;", Expected::Err("type mismatch")),
        ("-true", Expected::Err("unknown operator")),
        ("foobar", Expected::Err("identifier not found")),
    ];
    run_cases(engine, &cases);
}
