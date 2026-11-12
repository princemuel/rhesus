#![no_std]
extern crate alloc;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Vm,
    Eval,
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
    pub fn eval_line(&mut self, input: &str) -> Result<String, String> {
        match self.engine {
            Engine::Eval => {
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
