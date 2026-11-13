#![cfg(test)]
mod common;

use rhesus_core::Engine;

#[test]
#[ignore = "vm not implemented yet"]
fn vm_integer_arithmetic() { common::test_integer_arithmetic(Engine::Vm); }

#[test]
#[ignore = "vm not implemented yet"]
fn vm_boolean_expressions() { common::test_boolean_expressions(Engine::Vm); }

#[test]
#[ignore = "vm not implemented yet"]
fn vm_conditionals() { common::test_conditionals(Engine::Vm); }

#[test]
#[ignore = "vm not implemented yet"]
fn vm_closures_and_functions() { common::test_closures_and_functions(Engine::Vm); }

#[test]
#[ignore = "vm not implemented yet"]
fn vm_error_handling() { common::test_error_handling(Engine::Vm); }
