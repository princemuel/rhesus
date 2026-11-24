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
pub mod macros;
pub mod object;
pub mod parser;
pub mod prelude;
pub mod token;

use alloc::borrow::ToOwned as _;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use core::str::FromStr;

use crate::environment::Environment;
use crate::errors::Error;
use crate::evaluator::{DEFAULT_STACK_BUDGET, eval_program_with_budget};
use crate::host::Host;
use crate::lexer::Lexer;
use crate::macros::{define_macros, expand_macros_with_budget};
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
    macro_env: Environment<'static>,
    stack_budget: usize,
}

impl Session {
    #[must_use]
    pub fn new(engine: Engine) -> Self {
        Self {
            engine,
            env: Environment::new(),
            macro_env: Environment::new(),
            stack_budget: DEFAULT_STACK_BUDGET,
        }
    }

    /// How many bytes of native stack evaluating one line may use before it
    /// fails with a "stack overflow" error instead of crashing the process.
    ///
    /// The default (1 MiB) is safe on any thread with a 2 MiB stack or more.
    ///
    /// To run the session on a thread with a bigger stack, raise the stack
    /// budget to allow for deeper recursion, but keep it comfortably below the
    /// stack's size.
    #[must_use]
    pub const fn with_stack_budget(mut self, bytes: usize) -> Self {
        self.stack_budget = bytes;
        self
    }

    /// `Ok(None)` means "ran fine, nothing to print" (e.g. a `let`).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with one message per syntax error, or
    /// [`Error::Eval`] with the runtime error, so callers can present the two
    /// differently.
    pub fn eval_line(&mut self, input: &str, host: &mut dyn Host) -> Result<Option<String>, Error> {
        match self.engine {
            Engine::Eval => {
                let source: &'static str = Box::leak(input.to_owned().into_boxed_str());

                let program = Parser::new(Lexer::new(source))
                    .parse_program()
                    .map_err(|errs| Error::Parse(errs.iter().map(ToString::to_string).collect()))?;

                let program = define_macros(program, &self.macro_env);

                let program =
                    expand_macros_with_budget(program, &self.macro_env, host, self.stack_budget)
                        .map_err(|err| Error::Eval(err.to_string()))?;

                let value = eval_program_with_budget(&program, &self.env, host, self.stack_budget)
                    .map_err(|err| Error::Eval(err.to_string()))?;
                Ok(value.map(|v| v.to_string()))
            }
            Engine::Vm => {
                // TODO: Lexer -> Parser -> Compiler::compile -> VM::run
                Ok(Some(format!("[vm] compiled & ran {} bytes", input.len())))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::TestHost;

    /// One REPL session: every line shares state, like the real thing.
    struct Repl {
        session: Session,
        host: TestHost,
    }

    impl Repl {
        fn new() -> Self { Self { session: Session::new(Engine::Eval), host: TestHost::default() } }

        fn line(&mut self, input: &str) -> Result<Option<String>, Error> {
            self.session.eval_line(input, &mut self.host)
        }

        fn ok(&mut self, input: &str) -> Option<String> {
            self.line(input).unwrap_or_else(|err| panic!("{input:?} failed: {err}"))
        }
    }

    #[test]
    fn variables_persist_across_lines() {
        let mut repl = Repl::new();
        assert_eq!(repl.ok("let a = 5;"), None);
        assert_eq!(repl.ok("a * 2"), Some("10".into()));
        assert_eq!(repl.ok("let a = a + 1;"), None);
        assert_eq!(repl.ok("a"), Some("6".into()));
    }

    #[test]
    fn functions_outlive_the_line_that_defined_them() {
        // The AST borrows from each line's text, so this guards the leak.
        let mut repl = Repl::new();
        let _unused = repl.ok("let add = fn(x, y) { x + y };");
        assert_eq!(repl.ok("add(1, 2)"), Some("3".into()));

        drop(repl.ok("let adder = fn(x) { fn(y) { x + y } };"));
        drop(repl.ok("let add2 = adder(2);"));
        assert_eq!(repl.ok("add2(3)"), Some("5".into()));
    }

    #[test]
    fn macros_persist_across_lines() {
        let mut repl = Repl::new();
        let defined = repl.ok("let unless = macro(c, t, a) { quote(if (!(unquote(c))) { \
                               unquote(t) } else { unquote(a) }) };");
        assert_eq!(defined, None, "a macro definition prints nothing");

        assert_eq!(repl.ok("unless(10 > 5, 1, 2)"), Some("2".into()));
        assert_eq!(repl.ok("unless(10 < 5, 1, 2)"), Some("1".into()));
    }

    #[test]
    fn macros_and_variables_live_in_separate_environments() {
        let mut repl = Repl::new();
        drop(repl.ok("let m = macro() { quote(1) };"));
        let err = repl.line("m").unwrap_err();
        assert_eq!(
            err,
            Error::Eval("identifier not found: m".into()),
            "a macro is not a runtime variable"
        );
    }

    #[test]
    fn a_failed_line_does_not_break_the_session() {
        let mut repl = Repl::new();
        drop(repl.ok("let x = 1;"));
        assert_eq!(
            repl.line("x + true").unwrap_err(),
            Error::Eval("type mismatch: INTEGER + BOOLEAN".into())
        );
        assert!(matches!(repl.line("let = 5;"), Err(Error::Parse(_))));
        assert_eq!(repl.ok("x"), Some("1".into()));
    }

    #[test]
    fn parse_errors_are_all_reported() {
        let Err(Error::Parse(messages)) = Repl::new().line("let = 5; let y 6; 7") else {
            panic!("expected parse errors");
        };
        assert_eq!(messages.len(), 2, "one message per error: {messages:?}");

        let rendered = Error::Parse(messages).to_string();
        assert_eq!(rendered.lines().count(), 2, "one line per error: {rendered}");
    }

    #[test]
    fn runaway_recursion_is_an_error_and_the_session_survives() {
        let mut repl = Repl::new();
        drop(repl.ok("let f = fn() { f() };"));
        assert_eq!(
            repl.line("f()").unwrap_err(),
            Error::Eval("stack overflow: recursion too deep".into())
        );
        assert_eq!(repl.ok("1 + 1"), Some("2".into()));
    }

    #[test]
    fn the_stack_budget_is_configurable() {
        let mut session = Session::new(Engine::Eval).with_stack_budget(0);
        let err = session.eval_line("let f = fn() { 1 }; f()", &mut TestHost::default());
        assert_eq!(err, Err(Error::Eval("stack overflow: recursion too deep".into())));
    }

    #[test]
    fn an_empty_line_prints_nothing() {
        assert_eq!(Repl::new().ok(""), None);
        assert_eq!(Repl::new().ok("   "), None);
    }

    #[test]
    fn output_goes_to_the_host_and_the_value_is_returned() {
        let mut repl = Repl::new();
        assert_eq!(repl.ok(r#"puts("hi"); 7"#), Some("7".into()));
        assert_eq!(repl.host.output, ["hi"]);
    }

    #[test]
    fn a_top_level_return_ends_the_line() {
        assert_eq!(Repl::new().ok("return 1; 2"), Some("1".into()));
    }

    #[test]
    fn vm_engine_is_still_a_stub() {
        let mut session = Session::new(Engine::Vm);
        let out = session.eval_line("1 + 1", &mut TestHost::default()).unwrap();
        assert_eq!(out, Some("[vm] compiled & ran 5 bytes".into()));
    }

    #[test]
    fn engine_from_str() {
        assert_eq!("vm".parse(), Ok(Engine::Vm));
        assert_eq!("eval".parse(), Ok(Engine::Eval));
        let err = "jit".parse::<Engine>().unwrap_err();
        assert_eq!(err, "invalid engine 'jit', expected 'vm' or 'eval'");
    }
}
