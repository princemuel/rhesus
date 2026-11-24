#![cfg(test)]
mod common;
use rhesus_core::Engine;

const VM: Engine = Engine::Vm;

#[test]
#[ignore = "vm not implemented yet"]
fn vm_strings() { common::test_strings(VM); }

#[test]
#[ignore = "vm not implemented yet"]
fn vm_integer_arithmetic() { common::test_integer_arithmetic(VM); }

#[test]
#[ignore = "vm not implemented yet"]
fn vm_boolean_expressions() { common::test_boolean_expressions(VM); }

#[test]
#[ignore = "vm not implemented yet"]
fn vm_conditionals() { common::test_conditionals(VM); }

#[test]
#[ignore = "vm not implemented yet"]
fn vm_closures_and_functions() { common::test_closures_and_functions(VM); }

#[test]
#[ignore = "vm not implemented yet"]
fn vm_error_handling() { common::test_error_handling(VM); }

#[test]
#[ignore = "vm not implemented yet"]
fn vm_arrays_and_hashes() { common::test_arrays_and_hashes(VM); }

#[test]
#[ignore = "vm not implemented yet"]
fn vm_builtins() { common::test_builtins(VM); }
