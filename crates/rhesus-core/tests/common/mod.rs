//! Shared helpers for the engine-level tests. Every case uses the
//! public `Session::eval_line`, exactly as the CLI calls it, so the two engines
//! can run the same table of cases.
#![allow(dead_code, reason = "each integration-test crate uses a different subset")]

use rhesus_core::host::TestHost;
use rhesus_core::{Engine, Session};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Expected<'a> {
    Int(i64),
    Bool(bool),
    Str(&'a str),
    Null,
    /// The result is printed as this exact text (arrays, hashes, ...).
    Text(&'a str),
    /// The line succeeds but prints nothing (a `let`, a macro definition).
    Nothing,
    /// Matches a substring of the error message.
    Err(&'a str),
}
/// Evaluates one line on a fresh session and checks the result.
pub(crate) fn assert_eval(engine: Engine, input: &str, expected: Expected<'_>) {
    check(engine, &mut Session::new(engine), &mut TestHost::default(), input, expected);
}
/// Run a table of independent `(input, expected)` cases, each on a fresh
/// session.
pub(crate) fn run_cases(engine: Engine, cases: &[(&str, Expected<'_>)]) {
    for &(input, expected) in cases {
        assert_eval(engine, input, expected);
    }
}

/// Run the lines in order on ONE session, like a REPL transcript.
pub(crate) fn run_transcript(engine: Engine, lines: &[(&str, Expected<'_>)]) {
    let mut session = Session::new(engine);
    let mut host = TestHost::default();
    for &(input, expected) in lines {
        check(engine, &mut session, &mut host, input, expected);
    }
}

/// Like [`run_transcript`], but also returns everything the program printed.
pub(crate) fn run_transcript_with_output(
    engine: Engine,
    lines: &[(&str, Expected<'_>)],
) -> Vec<String> {
    let mut session = Session::new(engine);
    let mut host = TestHost::default();
    for &(input, expected) in lines {
        check(engine, &mut session, &mut host, input, expected);
    }
    host.output
}

fn check(
    engine: Engine,
    session: &mut Session,
    host: &mut TestHost,
    input: &str,
    expected: Expected<'_>,
) {
    let actual = session.eval_line(input, host);
    let ctx = || format!("[{engine:?}] on: {input}");

    match (actual, expected) {
        (Ok(Some(val)), Expected::Int(n)) => assert_eq!(val, n.to_string(), "{}", ctx()),
        (Ok(Some(val)), Expected::Bool(b)) => assert_eq!(val, b.to_string(), "{}", ctx()),
        (Ok(Some(val)), Expected::Str(s) | Expected::Text(s)) => assert_eq!(val, s, "{}", ctx()),
        (Ok(Some(val)), Expected::Null) => assert_eq!(val, "null", "{}", ctx()),
        (Ok(None), Expected::Nothing) => {}
        (Err(err), Expected::Err(substr)) => assert!(
            err.to_string().contains(substr),
            "expected an error containing {substr:?}, got {err:?}; {}",
            ctx()
        ),
        (Ok(val), Expected::Err(substr)) => {
            panic!("expected error {substr:?}, but got Ok({val:?}); {}", ctx())
        }
        (Err(err), exp) => panic!("expected {exp:?}, but got Err({err:?}); {}", ctx()),
        (Ok(val), exp) => panic!("expected {exp:?}, but got Ok({val:?}); {}", ctx()),
    }
}

pub(crate) fn test_strings(engine: Engine) {
    run_cases(engine, &[
        (r#""hello world""#, Expected::Str("hello world")),
        (r#""foo" + "bar""#, Expected::Str("foobar")),
        (r#""a\nb""#, Expected::Str("a\nb")),
        (r#""foo" - "bar""#, Expected::Err("unknown operator: STRING - STRING")),
    ]);
}
pub(crate) fn test_integer_arithmetic(engine: Engine) {
    run_cases(engine, &[
        ("5", Expected::Int(5)),
        ("10", Expected::Int(10)),
        ("-5", Expected::Int(-5)),
        ("-10", Expected::Int(-10)),
        ("5 + 5 + 5 + 5 - 10", Expected::Int(10)),
        ("2 * 2 * 2 * 2 * 2", Expected::Int(32)),
        ("5 * 2 + 10", Expected::Int(20)),
        ("5 + 2 * 10", Expected::Int(25)),
        ("50 / 2 * 2 + 10", Expected::Int(60)),
        ("(5 + 10 * 2 + 15 / 3) * 2 + -10", Expected::Int(50)),
    ]);
}
pub(crate) fn test_boolean_expressions(engine: Engine) {
    run_cases(engine, &[
        ("true", Expected::Bool(true)),
        ("false", Expected::Bool(false)),
        ("1 < 2", Expected::Bool(true)),
        ("1 > 2", Expected::Bool(false)),
        ("1 == 1", Expected::Bool(true)),
        ("1 != 1", Expected::Bool(false)),
        ("!true", Expected::Bool(false)),
        ("!!true", Expected::Bool(true)),
        ("!5", Expected::Bool(false)),
        ("(1 < 2) == true", Expected::Bool(true)),
    ]);
}
pub(crate) fn test_conditionals(engine: Engine) {
    run_cases(engine, &[
        ("if (true) { 10 }", Expected::Int(10)),
        ("if (false) { 10 }", Expected::Null),
        ("if (1) { 10 }", Expected::Int(10)),
        ("if (1 < 2) { 10 } else { 20 }", Expected::Int(10)),
        ("if (1 > 2) { 10 } else { 20 }", Expected::Int(20)),
    ]);
}
pub(crate) fn test_closures_and_functions(engine: Engine) {
    run_cases(engine, &[
        ("let identity = fn(x) { x; }; identity(5);", Expected::Int(5)),
        ("let add = fn(x, y) { x + y; }; add(5 + 5, add(5, 5));", Expected::Int(20)),
        ("fn(x) { x; }(5)", Expected::Int(5)),
        (
            "let newAdder = fn(x) { fn(y) { x + y }; };
         let addTwo = newAdder(2);
         addTwo(3);",
            Expected::Int(5),
        ),
    ]);
}
pub(crate) fn test_error_handling(engine: Engine) {
    run_cases(engine, &[
        ("5 + true;", Expected::Err("type mismatch: INTEGER + BOOLEAN")),
        ("-true", Expected::Err("unknown operator: -BOOLEAN")),
        ("true + false", Expected::Err("unknown operator: BOOLEAN + BOOLEAN")),
        ("foobar", Expected::Err("identifier not found: foobar")),
    ]);
}
pub(crate) fn test_arrays_and_hashes(engine: Engine) {
    run_cases(engine, &[
        ("[1, 2 * 2, 3 + 3]", Expected::Text("[1, 4, 6]")),
        ("[1, 2, 3][1]", Expected::Int(2)),
        ("[1, 2, 3][3]", Expected::Null),
        ("[1, 2, 3][-1]", Expected::Null),
        (r#"{"foo": 5}["foo"]"#, Expected::Int(5)),
        (r#"{"foo": 5}["bar"]"#, Expected::Null),
        ("{5: 5, true: 6}[true]", Expected::Int(6)),
        (r#"{"a": 1, "b": 2}"#, Expected::Text("{a: 1, b: 2}")),
        (r#"{"name": "Monkey"}[fn(x) { x }]"#, Expected::Err("unusable as hash key: FUNCTION")),
        ("5[0]", Expected::Err("index operator not supported: INTEGER")),
    ]);
}
pub(crate) fn test_builtins(engine: Engine) {
    run_cases(engine, &[
        (r#"len("four")"#, Expected::Int(4)),
        ("len([1, 2, 3])", Expected::Int(3)),
        ("first([1, 2, 3])", Expected::Int(1)),
        ("last([1, 2, 3])", Expected::Int(3)),
        ("rest([1, 2, 3])", Expected::Text("[2, 3]")),
        ("push([1], 2)", Expected::Text("[1, 2]")),
        ("first([])", Expected::Null),
        ("rest([])", Expected::Null),
        ("len(1)", Expected::Err("argument to `len` not supported, got INTEGER")),
        ("first(1)", Expected::Err("argument to `first` must be ARRAY, got INTEGER")),
        (r#"len("a", "b")"#, Expected::Err("wrong number of arguments. got=2, want=1")),
    ]);
}
