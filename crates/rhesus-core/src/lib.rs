#![no_std]
#![feature(core_io)]
#![feature(const_trait_impl)]
#![feature(const_cmp)]

extern crate alloc;

pub mod ast;
pub mod builtins;
pub mod environment;
pub mod errors;
pub mod evaluator;
pub mod host;
pub mod lexer;
pub mod object;
pub mod parser;
pub mod prelude;
pub mod token;

use alloc::borrow::ToOwned as _;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::str::FromStr;

use crate::environment::Environment;
use crate::evaluator::eval_program;
use crate::host::Host;
use crate::lexer::Lexer;
use crate::parser::Parser;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Vm,
    Eval,
}

impl FromStr for Engine {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "vm" => Ok(Self::Vm),
            "eval" => Ok(Self::Eval),
            other => Err(format!("invalid engine '{other}', expected 'vm' or 'eval'")),
        }
    }
}

#[derive(Debug)]
/// Persistent state across REPL lines (environments, globals, symbol tables)
pub struct Session {
    pub engine: Engine,
    env: Environment<'static>,
}

impl Session {
    #[must_use]
    pub fn new(engine: Engine) -> Self { Self { engine, env: Environment::new() } }

    /// `Ok(None)` means "ran fine, nothing to print" (e.g. a `let`).
    ///
    /// # Errors
    ///
    /// Returns the parse errors or the runtime error, rendered as text.
    pub fn eval_line(
        &mut self,
        input: &str,
        host: &mut dyn Host,
    ) -> Result<Option<String>, String> {
        match self.engine {
            Engine::Eval => {
                let source: &'static str = Box::leak(input.to_owned().into_boxed_str());

                let program = Parser::new(Lexer::new(source)).parse_program().map_err(|errs| {
                    errs.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n  ")
                })?;

                let value =
                    eval_program(&program, &self.env, host).map_err(|err| err.to_string())?;
                Ok(value.map(|v| v.to_string()))
            }
            Engine::Vm => {
                // TODO: Lexer -> Parser -> Compiler::compile -> VM::run
                Ok(Some(format!("[vm] compiled & ran {} bytes", input.len())))
            }
        }
    }
}
