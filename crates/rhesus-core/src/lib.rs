#![no_std]
#![feature(core_io)]
#![feature(const_trait_impl)]
#![feature(const_cmp)]
extern crate alloc;

pub mod ast;
pub mod errors;
pub mod evaluator;
pub mod lexer;
pub mod parser;
pub mod prelude;
pub mod token;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;
use core::io::Write;
use core::str::FromStr;

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
    _env: Rc<RefCell<()>>,
}

impl Session {
    #[must_use = ""]
    pub fn new(engine: Engine) -> Self { Self { engine, _env: Rc::new(RefCell::new(())) } }

    /// .
    ///
    /// # Errors
    ///
    /// This function will return an error if .
    pub fn eval_line<W: Write>(&mut self, input: &str, _out: &mut W) -> Result<String, String> {
        match self.engine {
            Engine::Eval => {
                let program = Parser::new(Lexer::new(input)).parse_program().map_err(|errors| {
                    errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n  ")
                })?;

                // TODO: Evaluator::eval(program, &mut self.env)
                Ok(program.to_string())
            }
            Engine::Vm => {
                // TODO: Lexer -> Parser -> Compiler::compile -> VM::run
                Ok(format!("[vm] compiled & ran {} bytes", input.len()))
            }
        }
    }
}
