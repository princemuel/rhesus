#![no_std]
#![feature(core_io)]
#![feature(const_trait_impl)]
#![feature(const_cmp)]
extern crate alloc;

pub mod lexer;
pub mod prelude;
pub mod token;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;
use core::io::Write;
use core::str::FromStr;

use crate::lexer::Lexer;

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
    pub fn eval_line<W: Write>(&mut self, input: &str, out: &mut W) -> Result<String, String> {
        match self.engine {
            Engine::Eval => {
                for token in Lexer::new(input) {
                    writeln!(out, "{token:?}").map_err(|err| format!("{err:?}"))?;
                }

                // TODO: Lexer -> Parser -> Evaluator::eval(program, &mut
                // self.env)
                Ok(format!("[eval] parsed {} bytes", input.len()))
            }
            Engine::Vm => {
                // TODO: Lexer -> Parser -> Compiler::compile -> VM::run
                Ok(format!("[vm] compiled & ran {} bytes", input.len()))
            }
        }
    }
}
