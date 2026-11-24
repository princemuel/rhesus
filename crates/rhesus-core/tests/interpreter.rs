#![cfg(test)]
mod common;
use rhesus_core::Engine;

use crate::common::{Expected, run_transcript, run_transcript_with_output};
const EVAL: Engine = Engine::Eval;

#[test]
fn eval_strings() { common::test_strings(EVAL); }

#[test]
fn eval_integer_arithmetic() { common::test_integer_arithmetic(EVAL); }

#[test]
fn eval_boolean_expressions() { common::test_boolean_expressions(EVAL); }

#[test]
fn eval_conditionals() { common::test_conditionals(EVAL); }

#[test]
fn eval_closures_and_functions() { common::test_closures_and_functions(EVAL); }

#[test]
fn eval_error_handling() { common::test_error_handling(EVAL); }

#[test]
fn eval_arrays_and_hashes() { common::test_arrays_and_hashes(EVAL); }

#[test]
fn eval_builtins() { common::test_builtins(EVAL); }

#[test]
fn repl_transcript_keeps_state_between_lines() {
    run_transcript(EVAL, &[
        ("let a = 5;", Expected::Nothing),
        ("let double = fn(x) { x * 2 };", Expected::Nothing),
        ("double(a)", Expected::Int(10)),
        ("let a = 100;", Expected::Nothing),
        ("double(a)", Expected::Int(200)),
        ("a + true", Expected::Err("type mismatch")),
        ("double(a)", Expected::Int(200)),
    ]);
}

#[test]
fn repl_transcript_with_recursion_and_collections() {
    run_transcript(EVAL, &[
        (
            "let fib = fn(n) { if (n < 2) { n } else { fib(n - 1) + fib(n - 2) } };",
            Expected::Nothing,
        ),
        ("fib(10)", Expected::Int(55)),
        ("let nums = [1, 2, 3, 4];", Expected::Nothing),
        (
            "let sum = fn(arr) { if (len(arr) == 0) { 0 } else { first(arr) + sum(rest(arr)) } };",
            Expected::Nothing,
        ),
        ("sum(nums)", Expected::Int(10)),
        ("nums", Expected::Text("[1, 2, 3, 4]")),
    ]);
}

#[test]
fn eval_quote_and_unquote() {
    run_transcript(EVAL, &[
        ("quote(5 + 8)", Expected::Text("QUOTE((+ 5 8))")),
        ("quote(unquote(4 + 4) + 8)", Expected::Text("QUOTE((+ 8 8))")),
        ("quote(unquote(quote(4 + 4)))", Expected::Text("QUOTE((+ 4 4))")),
    ]);
}

#[test]
fn macros_expand_before_evaluation() {
    run_transcript(EVAL, &[
        ("let reverse = macro(a, b) { quote(unquote(b) - unquote(a)) };", Expected::Nothing),
        ("reverse(2 + 2, 10 - 5)", Expected::Int(1)),
    ]);
}

#[test]
fn unless_only_evaluates_the_branch_it_takes() {
    let printed = run_transcript_with_output(EVAL, &[
        (
            "let unless = macro(condition, consequence, alternative) {
             quote(if (!(unquote(condition))) {
                 unquote(consequence);
             } else {
                 unquote(alternative);
             });
         };",
            Expected::Nothing,
        ),
        (r#"unless(10 > 5, puts("not greater"), puts("greater"))"#, Expected::Null),
        (r#"unless(10 < 5, puts("not greater"), puts("greater"))"#, Expected::Null),
    ]);
    assert_eq!(printed, ["greater", "not greater"]);
}

#[test]
fn macro_errors_are_reported_not_panicked() {
    run_transcript(EVAL, &[
        ("let bad = macro() { 1 };", Expected::Nothing),
        ("bad()", Expected::Err("macro must return a QUOTE, got INTEGER")),
        ("let needs_two = macro(a, b) { quote(1) };", Expected::Nothing),
        ("needs_two(1)", Expected::Err("wrong number of arguments. got=1, want=2")),
        ("quote()", Expected::Err("wrong number of arguments. got=0, want=1")),
        ("1 + 1", Expected::Int(2)),
    ]);
}

#[test]
fn identifiers_with_digits() {
    run_transcript(EVAL, &[
        ("let x1 = 1; let x2 = 2;", Expected::Nothing),
        ("x1 + x2", Expected::Int(3)),
    ]);
}
