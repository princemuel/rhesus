#![cfg(test)]
mod common;

use rhesus_core::Engine;

#[test]
fn eval_integer_arithmetic() { common::test_integer_arithmetic(Engine::Eval); }

#[test]
fn eval_boolean_expressions() { common::test_boolean_expressions(Engine::Eval); }

#[test]
fn eval_conditionals() { common::test_conditionals(Engine::Eval); }

#[test]
fn eval_closures_and_functions() { common::test_closures_and_functions(Engine::Eval); }

#[test]
fn eval_error_handling() { common::test_error_handling(Engine::Eval); }
