use alloc::borrow::Cow;
use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::{fmt, hint, ptr};

use crate::ast::{Block, Expr, InfixOp as Op, PrefixOp, Stmt};
use crate::builtins;
use crate::environment::Environment;
use crate::errors::EvalError;
use crate::host::Host;
use crate::macros::convert_obj_to_expr;
use crate::object::{Function, HashKey, Map, Object};

/// How much native stack evaluation may use by default: 1 MiB, which is safe on
/// any thread of 2 MiB or more (the default for spawned threads and tests).
///
/// A caller that runs the interpreter on a bigger stack can allow deeper
/// recursion with
/// [`Session::with_stack_budget`][`crate::Session::with_stack_budget`], or
/// [`Evaluator::with_stack_budget`] when driving an [`Evaluator`] directly.
pub const DEFAULT_STACK_BUDGET: usize = 1024 * 1024;

/// A tree-walking evaluator.
///
/// ```
/// use rhesus_core::environment::Environment;
/// use rhesus_core::evaluator::Evaluator;
/// use rhesus_core::host::TestHost;
/// use rhesus_core::lexer::Lexer;
/// use rhesus_core::parser::Parser;
///
/// let program = Parser::new(Lexer::new("let x = 20; x + 22")).parse_program().unwrap();
/// let mut host = TestHost::default();
/// let value = Evaluator::new(&mut host)
///     .with_stack_budget(8 * 1024 * 1024)
///     .eval(&program, &Environment::new())
///     .unwrap();
/// assert_eq!(value.unwrap().to_string(), "42");
/// ```
pub struct Evaluator<'h> {
    host: &'h mut dyn Host,
    stack: StackGuard,
}

impl fmt::Debug for Evaluator<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Evaluator").field("stack", &self.stack).finish_non_exhaustive()
    }
}

impl<'h> Evaluator<'h> {
    /// An evaluator that does its I/O through `host` and uses at most
    /// [`DEFAULT_STACK_BUDGET`] bytes of native stack per run.
    #[must_use]
    pub fn new(host: &'h mut dyn Host) -> Self {
        Self { host, stack: StackGuard::new(DEFAULT_STACK_BUDGET) }
    }

    /// How many bytes of native stack one run may use before it fails with
    /// [`EvalError::StackOverflow`] instead of crashing the process.
    ///
    /// Keep it comfortably below the size of the stack the evaluator runs on.
    #[must_use]
    pub const fn with_stack_budget(mut self, bytes: usize) -> Self {
        self.stack.budget = bytes;
        self
    }

    /// Runs a whole program.
    ///
    /// A result value of `None` means the last statement produced no
    /// value (a `let`, or an empty program), so the REPL prints nothing.
    ///
    /// The stack budget is measured from where each call to `eval` starts, so
    /// an evaluator can be reused, and a run that overflows leaves nothing
    /// behind for the next one.
    ///
    /// # Errors
    ///
    /// Returns the first runtime error, including [`EvalError::StackOverflow`]
    /// if evaluation needs more than the stack budget. A top-level `return`
    /// just ends the program with its value.
    pub fn eval<'a>(
        &mut self,
        program: &Block<'a>,
        env: &Environment<'a>,
    ) -> Result<Option<Object<'a>>, EvalError<'a>> {
        self.stack.reset();

        match self.eval_block(program, env) {
            Ok(value) => Ok(value),
            Err(Signal::Return(value)) => Ok(Some(value)),
            Err(Signal::Error(err)) => Err(err),
        }
    }

    /// Blocks don't open a new scope. Only function calls do.
    fn eval_block<'a>(
        &mut self,
        block: &Block<'a>,
        env: &Environment<'a>,
    ) -> Result<Option<Object<'a>>, Signal<'a>> {
        block.0.iter().try_fold(None, |_, stmt| self.eval_stmt(stmt, env))
    }

    fn eval_stmt<'a>(
        &mut self,
        stmt: &Stmt<'a>,
        env: &Environment<'a>,
    ) -> Result<Option<Object<'a>>, Signal<'a>> {
        match stmt {
            Stmt::Expr(expr) => self.eval_expr(expr, env).map(Some),
            Stmt::Return(expr) => Err(Signal::Return(self.eval_expr(expr, env)?)),
            Stmt::Let { name, value } => {
                let value = match self.eval_expr(value, env)? {
                    Object::Function(func) if func.name.is_none() => {
                        let parent = if func.env.ptr_eq(env) {
                            env.split_for_closure()
                        } else {
                            func.env.clone()
                        };

                        Object::Function(Rc::new(Function {
                            name: Some(name),
                            params: Rc::clone(&func.params),
                            body: Rc::clone(&func.body),
                            env: parent,
                        }))
                    }

                    Object::Macro(mac) if mac.name.is_none() => {
                        let parent = if mac.env.ptr_eq(env) {
                            env.split_for_closure()
                        } else {
                            mac.env.clone()
                        };

                        Object::Macro(Rc::new(Function {
                            name: Some(name),
                            params: Rc::clone(&mac.params),
                            body: Rc::clone(&mac.body),
                            env: parent,
                        }))
                    }
                    other => other,
                };
                env.set(name, value);
                Ok(None)
            }
        }
    }

    /// With `quote(expr)`, the argument is *not* evaluated and becomes a
    /// [`Object::Quote`].
    ///
    /// Only `unquote(..)` calls inside it are evaluated, and
    /// their results spliced in.
    fn eval_quote<'a>(
        &mut self,
        args: &[Expr<'a>],
        env: &Environment<'a>,
    ) -> Result<Object<'a>, Signal<'a>> {
        let [arg] = args else {
            return Err(EvalError::WrongArgumentCount { expected: 1, got: args.len() }.into());
        };

        let node = self.eval_unquote_calls(arg, env)?;
        Ok(Object::Quote(Rc::new(node)))
    }

    fn eval_unquote_calls<'a>(
        &mut self,
        expr: &Expr<'a>,
        env: &Environment<'a>,
    ) -> Result<Expr<'a>, Signal<'a>> {
        expr.clone().try_modify(&mut |node| -> Result<Expr<'a>, Signal<'a>> {
            match node {
                Expr::Call { function, args } if is_ident(&function, "unquote") => {
                    let [arg] = <[Expr<'a>; 1]>::try_from(args).map_err(|args| {
                        EvalError::WrongArgumentCount { expected: 1, got: args.len() }
                    })?;
                    Ok(convert_obj_to_expr(self.eval_expr(&arg, env)?)?)
                }
                expr => Ok(expr),
            }
        })
    }

    fn eval_expr<'a>(
        &mut self,
        expr: &Expr<'a>,
        env: &Environment<'a>,
    ) -> Result<Object<'a>, Signal<'a>> {
        self.stack.check()?; // it's important to call this here, early in the fn

        match expr {
            Expr::Int(n) => Ok(Object::Integer(*n)),
            Expr::Boolean(b) => Ok(Object::Boolean(*b)),
            Expr::String(s) => Ok(Object::String(s.clone())),
            // Variables shadow builtins
            Expr::Ident(name) => env
                .get(name)
                .or_else(|| builtins::lookup(name))
                .ok_or_else(|| Signal::from(EvalError::IdentifierNotFound(name))),
            Expr::Array(items) => Ok(self.eval_list(items, env)?.into()),
            Expr::Hash(pairs) => self.eval_hash(pairs, env),
            Expr::Index { lhs, index } => {
                let lhs = self.eval_expr(lhs, env)?;
                let index = self.eval_expr(index, env)?;
                Ok(eval_index(&lhs, &index)?)
            }
            Expr::Prefix { op, rhs } => {
                let rhs = self.eval_expr(rhs, env)?;
                Ok(eval_prefix(*op, &rhs)?)
            }
            Expr::Infix { lhs, op, rhs } => {
                let lhs = self.eval_expr(lhs, env)?;
                let rhs = self.eval_expr(rhs, env)?;
                Ok(eval_infix(*op, &lhs, &rhs)?)
            }
            Expr::If { condition, consequence, alternative } => {
                let branch = if self.eval_expr(condition, env)?.is_truthy() {
                    Some(consequence)
                } else {
                    alternative.as_ref()
                };

                match branch {
                    Some(block) => Ok(self.eval_block(block, env)?.unwrap_or_default()),
                    None => Ok(Object::Null),
                }
            }
            Expr::Function { params, body } => Ok(Object::Function(closure(params, body, env))),
            Expr::Macro { params, body } => Ok(Object::Macro(closure(params, body, env))),
            // `quote` is a special form: its argument must NOT be evaluated
            // first.
            Expr::Call { function, args } if is_ident(function, "quote") => {
                self.eval_quote(args, env)
            }
            Expr::Call { function, args } => {
                let callee = self.eval_expr(function, env)?;
                let args = self.eval_list(args, env)?;
                self.apply_function(&callee, args)
            }
        }
    }

    fn eval_list<'a>(
        &mut self,
        exprs: &[Expr<'a>],
        env: &Environment<'a>,
    ) -> Result<Vec<Object<'a>>, Signal<'a>> {
        exprs.iter().map(|expr| self.eval_expr(expr, env)).collect()
    }

    fn eval_hash<'a>(
        &mut self,
        pairs: &[(Expr<'a>, Expr<'a>)],
        env: &Environment<'a>,
    ) -> Result<Object<'a>, Signal<'a>> {
        let map = pairs
            .iter()
            .map(|(key, value)| {
                let key = HashKey::try_from(&self.eval_expr(key, env)?)?;
                Ok((key, self.eval_expr(value, env)?))
            })
            .collect::<Result<Map<_, _>, Signal<'a>>>()?;

        Ok(map.into())
    }

    fn apply_function<'a>(
        &mut self,
        callee: &Object<'a>,
        args: Vec<Object<'a>>,
    ) -> Result<Object<'a>, Signal<'a>> {
        match callee {
            Object::Function(func) => {
                let scope = bind_params_to_env(func, args)?;
                Ok(self.eval_func_or_macro_body(func, &scope)?)
            }
            Object::Builtin(builtin) => Ok(builtin(&args, self.host)?),
            other => Err(EvalError::NotAFunction(other.type_name()).into()),
        }
    }

    /// Runs a body in `scope`. A `return` ends the call here and goes no
    /// further.
    pub(crate) fn eval_func_or_macro_body<'a>(
        &mut self,
        func: &Function<'a>,
        scope: &Environment<'a>,
    ) -> Result<Object<'a>, EvalError<'a>> {
        match self.eval_block(&func.body, scope) {
            Ok(value) => Ok(value.unwrap_or_default()),
            Err(Signal::Return(value)) => Ok(value),
            Err(Signal::Error(err)) => Err(err),
        }
    }
}

/// Tracks how much native stack a run has used, and trips before it runs out.
///
/// The interpreter recurses once per nested expression and function call, so
/// a runaway program (`let f = fn() { f() }; f()`) would otherwise crash the
/// whole process. Instead, [`check`](Self::check) turns it into a catchable
/// [`EvalError::StackOverflow`].
///
/// It works by comparing the address of a local variable with the one taken
/// when the run started, so it needs no cooperation from the rest of the
/// evaluator and costs two pointer reads per check.
#[derive(Clone, Copy, Debug)]
struct StackGuard {
    /// Where the stack was when the run started.
    base: usize,
    /// How many bytes deeper than `base` the stack may grow.
    budget: usize,
}

impl StackGuard {
    fn new(budget: usize) -> Self { Self { base: Self::address(), budget } }

    /// Starts measuring from here: call it at the start of each run.
    fn reset(&mut self) { self.base = Self::address(); }

    fn check<'a>(&self) -> Result<(), EvalError<'a>> {
        // it doesn't matter which way the stack grows.
        if Self::address().abs_diff(self.base) > self.budget {
            return Err(EvalError::StackOverflow);
        }
        Ok(())
    }

    /// An address inside the current stack frame: a stand-in for the stack
    /// pointer.
    ///
    /// [`black_box`][`core::hint::black_box`] is essential. Without it the
    /// compiler sees a local that is never read or written, and is free to drop
    /// it or pick any address for it, for example a constant. That would make
    /// every measurement come out the same, so the budget would never trip and
    /// runaway recursion would crash after all.
    #[inline(never)]
    fn address() -> usize {
        let marker = 0u8;
        hint::black_box(ptr::from_ref(&marker)).addr()
    }
}

/// Why evaluation lhs the normal path. `?` propagates both kinds.
pub(crate) enum Signal<'a> {
    Return(Object<'a>),
    Error(EvalError<'a>),
}

impl<'a> From<EvalError<'a>> for Signal<'a> {
    fn from(err: EvalError<'a>) -> Self { Self::Error(err) }
}

/// Out-of-range and negative array indexes, and missing hash keys, give `null`.
fn eval_index<'a>(lhs: &Object<'a>, index: &Object<'a>) -> Result<Object<'a>, EvalError<'a>> {
    match (lhs, index) {
        (Object::Array(items), Object::Integer(i)) => {
            Ok(usize::try_from(*i).ok().and_then(|idx| items.get(idx)).cloned().unwrap_or_default())
        }
        (Object::Hash(map), key) => {
            Ok(map.get(&HashKey::try_from(key)?).cloned().unwrap_or_default())
        }
        _ => Err(EvalError::IndexNotSupported(lhs.type_name())),
    }
}

fn eval_prefix<'a>(op: PrefixOp, rhs: &Object<'a>) -> Result<Object<'a>, EvalError<'a>> {
    match (op, rhs) {
        (PrefixOp::Bang, _) => Ok(Object::Boolean(!rhs.is_truthy())),
        (PrefixOp::Minus, Object::Integer(n)) => {
            n.checked_neg().map(Object::Integer).ok_or(EvalError::IntegerOverflow)
        }
        (PrefixOp::Minus, other) => {
            Err(EvalError::UnknownPrefixOperator { op, operand: other.type_name() })
        }
    }
}

fn eval_infix<'a>(op: Op, lhs: &Object<'a>, rhs: &Object<'a>) -> Result<Object<'a>, EvalError<'a>> {
    match (op, lhs, rhs) {
        (_, Object::Integer(lhs), Object::Integer(rhs)) => eval_int_infix(op, *lhs, *rhs),
        (Op::Plus, Object::String(lhs), Object::String(rhs)) => {
            Ok(Object::String(Cow::Owned(format!("{lhs}{rhs}"))))
        }
        (Op::Eq, ..) => Ok(Object::Boolean(lhs == rhs)),
        (Op::NotEq, ..) => Ok(Object::Boolean(lhs != rhs)),
        _ if lhs.type_name() != rhs.type_name() => {
            Err(EvalError::TypeMismatch { lhs: lhs.type_name(), op, rhs: rhs.type_name() })
        }
        _ => {
            Err(EvalError::UnknownInfixOperator { lhs: lhs.type_name(), op, rhs: rhs.type_name() })
        }
    }
}

fn eval_int_infix<'a>(op: Op, lhs: i64, rhs: i64) -> Result<Object<'a>, EvalError<'a>> {
    let int = |result: Option<i64>| result.map(Object::Integer).ok_or(EvalError::IntegerOverflow);

    match op {
        Op::Plus => int(lhs.checked_add(rhs)),
        Op::Minus => int(lhs.checked_sub(rhs)),
        Op::Asterisk => int(lhs.checked_mul(rhs)),
        Op::Slash if rhs == 0 => Err(EvalError::DivisionByZero),
        Op::Slash => int(lhs.checked_div(rhs)),
        Op::Lt => Ok(Object::Boolean(lhs < rhs)),
        Op::Gt => Ok(Object::Boolean(lhs > rhs)),
        Op::Eq => Ok(Object::Boolean(lhs == rhs)),
        Op::NotEq => Ok(Object::Boolean(lhs != rhs)),
    }
}

fn closure<'a>(
    params: &Rc<[&'a str]>,
    body: &Rc<Block<'a>>,
    env: &Environment<'a>,
) -> Rc<Function<'a>> {
    Rc::new(Function {
        name: None,
        params: Rc::clone(params),
        body: Rc::clone(body),
        env: env.clone(),
    })
}

pub(crate) fn is_ident(expr: &Expr<'_>, name: &str) -> bool {
    matches!(expr, Expr::Ident(ident) if *ident == name)
}

/// A fresh scope, enclosed by the function's own, with each parameter bound.
pub(crate) fn bind_params_to_env<'a>(
    func: &Rc<Function<'a>>,
    args: Vec<Object<'a>>,
) -> Result<Environment<'a>, EvalError<'a>> {
    if func.params.len() != args.len() {
        return Err(EvalError::WrongArgumentCount { expected: func.params.len(), got: args.len() });
    }

    let scope = Environment::new_enclosed(&func.env);

    if let Some(name) = func.name {
        scope.set(name, Object::Function(Rc::clone(func)));
    }

    for (name, arg) in func.params.iter().zip(args) {
        scope.set(name, arg);
    }

    Ok(scope)
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString as _};
    use alloc::vec;

    use super::*;
    use crate::errors::HostError;
    use crate::host::{TestHost, WriterHost};
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    /// A host whose every capability fails with an I/O error.
    struct FailingHost;

    impl Host for FailingHost {
        fn write_line(&mut self, _text: &str) -> Result<(), HostError> { Err(HostError::Io) }

        fn read_line(&mut self) -> Result<Option<String>, HostError> { Err(HostError::Io) }

        fn now_millis(&self) -> Result<i64, HostError> { Err(HostError::Io) }
    }

    fn parse(input: &str) -> Block<'_> {
        Parser::new(Lexer::new(input))
            .parse_program()
            .unwrap_or_else(|errs| panic!("parse errors for {input:?}: {errs:#?}"))
    }

    fn eval(input: &str) -> Result<Option<Object<'_>>, EvalError<'_>> {
        Evaluator::new(&mut WriterHost(core::io::sink())).eval(&parse(input), &Environment::new())
    }

    fn eval_with<'a>(
        input: &'a str,
        host: &mut dyn Host,
    ) -> Result<Option<Object<'a>>, EvalError<'a>> {
        Evaluator::new(host).eval(&parse(input), &Environment::new())
    }

    fn assert_value<'a>(input: &'a str, want: &Object<'a>) {
        assert_eq!(&value(input), want, "input: {input}");
    }

    fn assert_error(input: &str, want: &str) {
        let err = eval(input).expect_err("expected an error");
        assert_eq!(err.to_string(), want, "input: {input}");
    }

    fn assert_host_error(input: &str, host: &mut dyn Host, want: &str) {
        let err = eval_with(input, host).expect_err("expected an error");
        assert_eq!(err.to_string(), want, "input: {input}");
    }

    fn value(input: &str) -> Object<'_> { eval(input).expect("eval error").expect("no value") }

    fn str_obj(s: &str) -> Object<'_> { Object::String(Cow::Borrowed(s)) }

    fn array<'a>(items: impl IntoIterator<Item = i64>) -> Object<'a> {
        Object::Array(items.into_iter().map(Object::Integer).collect())
    }
    fn int<'a>(n: i64) -> Object<'a> { Object::Integer(n) }
    fn boolean<'a>(b: bool) -> Object<'a> { Object::Boolean(b) }

    #[test]
    fn eval_integer_expression() {
        let tests = [
            ("5", 5),
            ("10", 10),
            ("-5", -5),
            ("-10", -10),
            ("5 + 5 + 5 + 5 - 10", 10),
            ("2 * 2 * 2 * 2 * 2", 32),
            ("-50 + 100 + -50", 0),
            ("5 * 2 + 10", 20),
            ("5 + 2 * 10", 25),
            ("20 + 2 * -10", 0),
            ("50 / 2 * 2 + 10", 60),
            ("2 * (5 + 10)", 30),
            ("3 * 3 * 3 + 10", 37),
            ("3 * (3 * 3) + 10", 37),
            ("(5 + 10 * 2 + 15 / 3) * 2 + -10", 50),
        ];
        for (input, want) in tests {
            assert_eq!(value(input), int(want), "input: {input}");
        }
    }

    #[test]
    fn eval_boolean_expression() {
        let tests = [
            ("true", true),
            ("false", false),
            ("1 < 2", true),
            ("1 > 2", false),
            ("1 < 1", false),
            ("1 > 1", false),
            ("1 == 1", true),
            ("1 != 1", false),
            ("1 == 2", false),
            ("1 != 2", true),
            ("true == true", true),
            ("false == false", true),
            ("true == false", false),
            ("true != false", true),
            ("false != true", true),
            ("(1 < 2) == true", true),
            ("(1 < 2) == false", false),
            ("(1 > 2) == true", false),
            ("(1 > 2) == false", true),
        ];
        for (input, want) in tests {
            assert_eq!(value(input), boolean(want), "input: {input}");
        }
    }

    #[test]
    fn bang_operator() {
        let tests = [
            ("!true", false),
            ("!false", true),
            ("!5", false),
            ("!!true", true),
            ("!!false", false),
            ("!!5", true),
        ];
        for (input, want) in tests {
            assert_eq!(value(input), boolean(want), "input: {input}");
        }
    }

    #[test]
    fn if_else_expressions() {
        let tests = [
            ("if (true) { 10 }", Some(10)),
            ("if (false) { 10 }", None),
            ("if (1) { 10 }", Some(10)),
            ("if (1 < 2) { 10 }", Some(10)),
            ("if (1 > 2) { 10 }", None),
            ("if (1 > 2) { 10 } else { 20 }", Some(20)),
            ("if (1 < 2) { 10 } else { 20 }", Some(10)),
        ];
        for (input, want) in tests {
            let want = want.map_or(Object::Null, Object::Integer);
            assert_eq!(value(input), want, "input: {input}");
        }
    }

    #[test]
    fn return_statements() {
        let tests = [
            ("return 10;", 10),
            ("return 10; 9;", 10),
            ("return 2 * 5; 9;", 10),
            ("9; return 2 * 5; 9;", 10),
            ("if (10 > 1) { return 10; }", 10),
            (
                "
if (10 > 1) {
  if (10 > 1) {
    return 10;
  }

  return 1;
}
",
                10,
            ),
            (
                "
let f = fn(x) {
  return x;
  x + 10;
};
f(10);",
                10,
            ),
            (
                "
let f = fn(x) {
   let result = x + 10;
   return result;
   return 10;
};
f(10);",
                20,
            ),
        ];
        for (input, want) in tests {
            assert_eq!(value(input), int(want), "input: {input}");
        }
    }

    #[test]
    fn error_handling() {
        let tests = [
            ("5 + true;", "type mismatch: INTEGER + BOOLEAN"),
            ("5 + true; 5;", "type mismatch: INTEGER + BOOLEAN"),
            ("-true", "unknown operator: -BOOLEAN"),
            ("true + false;", "unknown operator: BOOLEAN + BOOLEAN"),
            ("true + false + true + false;", "unknown operator: BOOLEAN + BOOLEAN"),
            ("5; true + false; 5", "unknown operator: BOOLEAN + BOOLEAN"),
            ("if (10 > 1) { true + false; }", "unknown operator: BOOLEAN + BOOLEAN"),
            (
                "
if (10 > 1) {
  if (10 > 1) {
    return true + false;
  }

  return 1;
}
",
                "unknown operator: BOOLEAN + BOOLEAN",
            ),
            ("foobar", "identifier not found: foobar"),
        ];
        for (input, want) in tests {
            let err = eval(input).expect_err("expected an error");
            assert_eq!(err.to_string(), want, "input: {input}");
        }
    }

    #[test]
    fn let_statements() {
        let tests = [
            ("let a = 5; a;", 5),
            ("let a = 5 * 5; a;", 25),
            ("let a = 5; let b = a; b;", 5),
            ("let a = 5; let b = a; let c = a + b + 5; c;", 15),
        ];
        for (input, want) in tests {
            assert_eq!(value(input), int(want), "input: {input}");
        }
    }

    #[test]
    fn function_object() {
        let Object::Function(func) = value("fn(x) { x + 2; };") else {
            panic!("object is not a function");
        };
        assert_eq!(*func.params, ["x"]);
        assert_eq!(func.body.to_string(), "(do (+ x 2))");
    }

    #[test]
    fn function_application() {
        let tests = [
            ("let identity = fn(x) { x; }; identity(5);", 5),
            ("let identity = fn(x) { return x; }; identity(5);", 5),
            ("let double = fn(x) { x * 2; }; double(5);", 10),
            ("let add = fn(x, y) { x + y; }; add(5, 5);", 10),
            ("let add = fn(x, y) { x + y; }; add(5 + 5, add(5, 5));", 20),
            ("fn(x) { x; }(5)", 5),
        ];
        for (input, want) in tests {
            assert_eq!(value(input), int(want), "input: {input}");
        }
    }

    #[test]
    fn enclosing_environments() {
        let input = "
let first = 10;
let second = 10;
let third = 10;

let ourFunction = fn(first) {
  let second = 20;

  first + second + third;
};

ourFunction(20) + first + second;";
        assert_eq!(value(input), int(70));
    }

    #[test]
    fn closures_capture_their_scope() {
        let input =
            "let newAdder = fn(x) { fn(y) { x + y } }; let addTwo = newAdder(2); addTwo(2);";
        assert_eq!(value(input), int(4));
    }

    #[test]
    fn statements_without_a_value_yield_none() {
        assert_eq!(eval("let a = 5;"), Ok(None));
        assert_eq!(eval(""), Ok(None));
    }

    #[test]
    fn mixed_type_equality_is_false_not_an_error() {
        assert_eq!(value("5 == true"), boolean(false));
        assert_eq!(value("5 != true"), boolean(true));
    }

    #[test]
    fn go_panics_become_errors() {
        let tests = [
            ("1 / 0", "division by zero"),
            ("9223372036854775807 + 1", "integer overflow"),
            ("let f = fn(x) { x }; f()", "wrong number of arguments. got=0, want=1"),
            ("let f = fn(x) { x }; f(1, 2)", "wrong number of arguments. got=2, want=1"),
            ("5(1)", "not a function: INTEGER"),
        ];
        for (input, want) in tests {
            let err = eval(input).expect_err("expected an error");
            assert_eq!(err.to_string(), want, "input: {input}");
        }
    }

    #[test]
    fn string_literal_and_concatenation() {
        assert_value(r#""Hello World!""#, &str_obj("Hello World!"));
        assert_value(r#""Hello" + " " + "World!""#, &str_obj("Hello World!"));
    }

    #[test]
    fn builtin_functions() {
        let values = [
            (r#"len("")"#, int(0)),
            (r#"len("four")"#, int(4)),
            (r#"len("hello world")"#, int(11)),
            ("len([1, 2, 3])", int(3)),
            ("len([])", int(0)),
            (r#"puts("hello", "world!")"#, Object::Null),
            ("first([1, 2, 3])", int(1)),
            ("first([])", Object::Null),
            ("last([1, 2, 3])", int(3)),
            ("last([])", Object::Null),
            ("rest([1, 2, 3])", array([2, 3])),
            ("rest([])", Object::Null),
            ("push([], 1)", array([1])),
        ];
        for (input, want) in &values {
            assert_value(input, want);
        }

        let errors = [
            ("len(1)", "argument to `len` not supported, got INTEGER"),
            (r#"len("one", "two")"#, "wrong number of arguments. got=2, want=1"),
            ("first(1)", "argument to `first` must be ARRAY, got INTEGER"),
            ("last(1)", "argument to `last` must be ARRAY, got INTEGER"),
            ("push(1, 1)", "argument to `push` must be ARRAY, got INTEGER"),
        ];
        for (input, want) in errors {
            assert_error(input, want);
        }
    }

    #[test]
    fn array_literals() { assert_value("[1, 2 * 2, 3 + 3]", &array([1, 4, 6])); }

    #[test]
    fn array_index_expressions() {
        let tests = [
            ("[1, 2, 3][0]", Some(1)),
            ("[1, 2, 3][1]", Some(2)),
            ("[1, 2, 3][2]", Some(3)),
            ("let i = 0; [1][i];", Some(1)),
            ("[1, 2, 3][1 + 1];", Some(3)),
            ("let myArray = [1, 2, 3]; myArray[2];", Some(3)),
            ("let myArray = [1, 2, 3]; myArray[0] + myArray[1] + myArray[2];", Some(6)),
            ("let myArray = [1, 2, 3]; let i = myArray[0]; myArray[i]", Some(2)),
            ("[1, 2, 3][3]", None),
            ("[1, 2, 3][-1]", None),
        ];
        for (input, want) in tests {
            assert_value(input, &want.map_or(Object::Null, Object::Integer));
        }
    }

    #[test]
    fn hash_literals() {
        let input = r#"
let two = "two";
{
  "one": 10 - 9,
  two: 1 + 1,
  "thr" + "ee": 6 / 2,
  4: 4,
  true: 5,
  false: 6
}"#;
        let Object::Hash(map) = value(input) else { panic!("object is not a hash") };

        let expected = [
            (HashKey::String("one".into()), 1),
            (HashKey::String("two".into()), 2),
            (HashKey::String("three".into()), 3),
            (HashKey::Integer(4), 4),
            (HashKey::Boolean(true), 5),
            (HashKey::Boolean(false), 6),
        ];
        assert_eq!(map.len(), expected.len());

        for (key, want) in expected {
            assert_eq!(map.get(&key), Some(&int(want)), "key: {key}");
        }
    }

    #[test]
    fn hash_index_expressions() {
        let tests = [
            (r#"{"foo": 5}["foo"]"#, Some(5)),
            (r#"{"foo": 5}["bar"]"#, None),
            (r#"let key = "foo"; {"foo": 5}[key]"#, Some(5)),
            (r#"{}["foo"]"#, None),
            ("{5: 5}[5]", Some(5)),
            ("{true: 5}[true]", Some(5)),
            ("{false: 5}[false]", Some(5)),
        ];
        for (input, want) in tests {
            assert_value(input, &want.map_or(Object::Null, Object::Integer));
        }
    }

    #[test]
    fn errors() {
        assert_error(r#""Hello" - "World""#, "unknown operator: STRING - STRING");
        assert_error(r#"{"name": "Monkey"}[fn(x) { x }];"#, "unusable as hash key: FUNCTION");
        assert_error("5[0]", "index operator not supported: INTEGER");
    }

    #[test]
    fn puts_writes_each_argument_on_its_own_line() {
        let mut host = TestHost::default();
        let result = eval_with(r#"puts("hi", 1, [1, 2])"#, &mut host);
        assert_eq!(result, Ok(Some(Object::Null)));
        assert_eq!(host.output, ["hi", "1", "[1, 2]"]);
    }

    #[test]
    fn input_returns_lines_then_null() {
        let mut host = TestHost { input: vec!["a".into()], ..TestHost::default() };
        let result = eval_with("[input(), input()]", &mut host);
        assert_eq!(result, Ok(Some(Object::from(vec!["a".into(), Object::Null]))));
    }

    #[test]
    fn input_replays_lines_in_order_then_null() {
        let mut host = TestHost { input: vec!["a".into(), "b".into()], ..TestHost::default() };
        let result = eval_with("[input(), input(), input()]", &mut host);
        assert_eq!(result, Ok(Some(Object::from(vec!["a".into(), "b".into(), Object::Null]))));
    }

    #[test]
    fn clock_reads_the_host_clock() {
        let mut host = TestHost { now: 1234, ..TestHost::default() };
        assert_eq!(eval_with("clock()", &mut host), Ok(Some(int(1234))));
    }

    #[test]
    fn unsupported_host_capabilities_are_errors() {
        assert_eq!(eval("input()"), Err(EvalError::Host(HostError::Unsupported)));
        assert_eq!(eval("clock()"), Err(EvalError::Host(HostError::Unsupported)));
    }

    #[test]
    fn host_io_errors_propagate_from_every_builtin() {
        for input in [r#"puts("hi")"#, "input()", "clock()"] {
            assert_host_error(input, &mut FailingHost, "host I/O error");
        }
    }

    #[test]
    fn host_errors_are_catchable_in_nested_calls() {
        // The error must travel out through function calls, not get swallowed.
        assert_host_error(
            r#"let f = fn() { puts("x"); 1 }; f() + 1"#,
            &mut FailingHost,
            "host I/O error",
        );
    }

    #[test]
    fn zero_arg_builtins_reject_arguments() {
        let mut host = TestHost::default();
        assert_host_error("input(1)", &mut host, "wrong number of arguments. got=1, want=0");
        assert_host_error("clock(1, 2)", &mut host, "wrong number of arguments. got=2, want=0");
    }

    #[test]
    fn puts_with_no_arguments_writes_nothing() {
        let mut host = TestHost::default();
        assert_eq!(eval_with("puts()", &mut host), Ok(Some(Object::Null)));
        assert_eq!(host.output, Vec::<String>::new());
    }

    #[test]
    fn output_before_a_runtime_error_is_kept() {
        let mut host = TestHost::default();
        assert_host_error(r#"puts("before"); 1 / 0"#, &mut host, "division by zero");
        assert_eq!(host.output, ["before"]);
    }

    #[test]
    fn host_state_persists_across_puts_calls() {
        let mut host = TestHost::default();
        drop(eval_with(r#"puts("one"); puts("two")"#, &mut host).unwrap());
        assert_eq!(host.output, ["one", "two"]);
    }

    #[test]
    fn writer_host_supports_output_only() {
        let mut host = WriterHost(core::io::sink());
        assert!(eval_with(r#"puts("ok")"#, &mut host).is_ok());
        assert_host_error("input()", &mut host, "operation not supported by host");
        assert_host_error("clock()", &mut host, "operation not supported by host");
    }

    #[test]
    fn equality_semantics() {
        let tests = [
            ("[1, 2] == [1, 2]", true),
            ("[1, 2] == [1, 3]", false),
            (r#"{"a": 1} == {"a": 1}"#, true),
            (r#""a" == "a""#, true),
            (r#""a" != "b""#, true),
            ("let f = fn() { 1 }; f == f", true),
            ("fn() { 1 } == fn() { 1 }", false),
            ("len == len", true),
            ("len == first", false),
            ("if (false) { 1 } == if (false) { 2 }", true),
        ];
        for (input, want) in tests {
            assert_eq!(value(input), boolean(want), "input: {input}");
        }
    }

    #[test]
    fn truthiness() {
        let tests = [
            ("if (0) { 1 } else { 2 }", 1),
            (r#"if ("") { 1 } else { 2 }"#, 1),
            ("if ([]) { 1 } else { 2 }", 1),
            ("if (if (false) { 1 }) { 1 } else { 2 }", 2),
        ];
        for (input, want) in tests {
            assert_eq!(value(input), int(want), "input: {input}");
        }
    }

    #[test]
    fn empty_blocks_and_bodies_evaluate_to_null() {
        assert_eq!(value("if (true) { }"), Object::Null);
        assert_eq!(value("fn() { }()"), Object::Null);
        assert_eq!(value("fn() { let a = 1; }()"), Object::Null);
    }

    #[test]
    fn return_does_not_leak_out_of_the_call() {
        // The book's evaluator can leak a ReturnValue through call arguments.
        assert_eq!(value("let f = fn() { return 5; 6 }; f() + 1"), int(6));
        assert_eq!(value("let f = fn() { return 5; 6 }; [f(), 1]"), array([5, 1]));
        assert_eq!(value("let f = fn() { return 5; 6 }; len([f()])"), int(1));
    }

    #[test]
    fn errors_stop_evaluation_immediately() {
        let mut host = TestHost::default();
        let result = eval_with(r#"puts("before"); 1 + true; puts("after")"#, &mut host);
        assert_eq!(result.unwrap_err().to_string(), "type mismatch: INTEGER + BOOLEAN");
        assert_eq!(host.output, ["before"]);
    }

    #[test]
    fn arguments_evaluate_lhs_to_rhs() {
        let mut host = TestHost::default();
        drop(eval_with(r#"[puts("a"), puts("b"), puts("c")]"#, &mut host));
        assert_eq!(host.output, ["a", "b", "c"]);
    }

    #[test]
    fn variables_shadow_builtins() {
        assert_eq!(value("let len = fn(x) { 42 }; len([1])"), int(42));
    }

    #[test]
    fn builtins_work_on_hash_and_nested_structures() {
        assert_eq!(value("len(rest(push([1, 2], 3)))"), int(2));
        assert_eq!(value("first(rest([[1, 2], [3, 4]]))[1]"), int(4));
        assert_eq!(value(r#"{"a": [1, 2]}["a"][1]"#), int(2));
    }

    #[test]
    fn push_and_rest_do_not_mutate_their_argument() {
        assert_eq!(value("let a = [1, 2]; let b = push(a, 3); len(a)"), int(2));
        assert_eq!(value("let a = [1, 2]; let b = rest(a); len(a)"), int(2));
    }

    #[test]
    fn unbounded_recursion_is_an_error_not_a_crash() {
        // Each shape costs a different amount of native stack per call; the
        // byte budget has to catch all of them before the (2 MiB) test thread's
        // stack runs out.
        let shapes = [
            "let f = fn() { f() }; f()",
            "let f = fn(n) { let a = [1, {\"k\": 2}, n]; 1 + (2 * f(n + 1)) }; f(0)",
            "let g = fn(x) { x }; let f = fn() { g(g(g(g(f())))) }; f()",
        ];
        for input in shapes {
            assert_error(input, "stack overflow: recursion too deep");
        }
    }

    #[test]
    fn a_stack_overflow_can_be_caught_and_the_session_continues() {
        let env = Environment::new();
        let host = &mut TestHost::default();
        let mut run = |src| Evaluator::new(host).eval(&parse(src), &env);

        assert_eq!(run("let f = fn() { f() };"), Ok(None));
        assert_eq!(run("f()"), Err(EvalError::StackOverflow));
        // The budget is per run, so nothing is lhs over from the failure.
        assert_eq!(run("let g = fn(x) { x }; g(5)"), Ok(Some(int(5))));
    }

    #[test]
    fn recursion_within_the_budget_works() {
        let count = "let count = fn(n) { if (n == 0) { 0 } else { 1 + count(n - 1) } }; count(40)";
        assert_eq!(value(count), int(40));
    }

    #[test]
    fn a_bigger_budget_on_a_bigger_stack_allows_deeper_recursion() {
        extern crate std;

        const STACK_SIZE: usize = 64 * 1024 * 1024;
        let src = "let count = fn(n) { if (n == 0) { 0 } else { 1 + count(n - 1) } }; count(1000)";
        assert_error(src, "stack overflow: recursion too deep");

        let result = std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || {
                let program = parse(src);
                let host = &mut TestHost::default();
                Evaluator::new(host)
                    .with_stack_budget(STACK_SIZE / 2)
                    .eval(&program, &Environment::new())
                    .map(|value| value.map(|v| v.to_string()))
                    .map_err(|err| err.to_string())
            })
            .expect("spawn")
            .join()
            .expect("the evaluator must not overflow the stack");
        assert_eq!(result, Ok(Some("1000".to_string())));
    }

    /// Recurse `levels` times, with a frame at least 512 bytes big,
    /// and return the stack address seen at the bottom.
    #[inline(never)]
    fn address_at_depth(levels: usize) -> usize {
        let padding = [0u8; 512];
        let address =
            if levels == 0 { StackGuard::address() } else { address_at_depth(levels - 1) };
        let _unused = hint::black_box(&padding); // keeps this frame alive across the call
        address
    }

    #[test]
    fn the_stack_address_really_moves_as_frames_pile_up() {
        // If the compiler ever turns `StackGuard::address` into a constant, the
        // stack budget silently stops working. This test fails first.
        let top = StackGuard::address();
        let bottom = address_at_depth(16);
        assert!(
            bottom.abs_diff(top) >= 16 * 512,
            "StackGuard::address barely moved: {top:#x} vs {bottom:#x}"
        );
    }

    #[test]
    fn the_guard_trips_only_once_the_stack_is_deeper_than_its_budget() {
        let guard = StackGuard::new(16 * 1024);

        assert_eq!(guard.check(), Ok(()));
        // 8 frames are at least 4 KiB down, 64 frames at least 32 KiB.
        assert_eq!(at_depth(8, &mut || guard.check()), Ok(()));
        assert_eq!(at_depth(64, &mut || guard.check()), Err(EvalError::StackOverflow));
    }

    #[test]
    fn resetting_the_guard_measures_from_the_new_spot() {
        let mut guard = StackGuard::new(16 * 1024);

        assert_eq!(at_depth(64, &mut || guard.check()), Err(EvalError::StackOverflow));
        let result = at_depth(64, &mut || {
            guard.reset();
            guard.check()
        });
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn a_new_evaluator_uses_the_default_budget() {
        let host = &mut TestHost::default();
        assert_eq!(Evaluator::new(host).stack.budget, DEFAULT_STACK_BUDGET);
    }

    #[test]
    fn one_evaluator_can_run_several_programs() {
        let env = Environment::new();
        let host = &mut TestHost::default();
        let mut evaluator = Evaluator::new(host);

        assert_eq!(evaluator.eval(&parse("let f = fn() { f() };"), &env), Ok(None));
        assert_eq!(evaluator.eval(&parse("f()"), &env), Err(EvalError::StackOverflow));
        // A failed run leaves nothing behind: the next one starts afresh.
        assert_eq!(evaluator.eval(&parse("let g = fn(x) { x }; g(5)"), &env), Ok(Some(int(5))));
    }

    /// Recurses `levels` frames deep (each at least 512 bytes), then runs `f`
    /// from there.
    #[inline(never)]
    fn at_depth<R>(levels: usize, f: &mut dyn FnMut() -> R) -> R {
        let padding = [0u8; 512];
        let result = if levels == 0 { f() } else { at_depth(levels - 1, f) };
        let _unused = hint::black_box(&padding); // keeps this frame alive across the call
        result
    }

    #[test]
    fn each_run_is_measured_from_where_it_starts_not_where_the_evaluator_was_built() {
        let host = &mut TestHost::default();
        let program = parse("1 + 1");

        // Built here, but run ~200 KiB further down the stack: far more than
        // the 64 KiB budget, which only holds if `eval` re-measures its base.
        let mut evaluator = Evaluator::new(host).with_stack_budget(64 * 1024);
        let result = at_depth(400, &mut || evaluator.eval(&program, &Environment::new()));
        assert_eq!(result, Ok(Some(int(2))));
    }

    #[test]
    fn a_tiny_budget_fails_fast() {
        let program = parse("let f = fn(x) { x }; f(1)");
        let host = &mut TestHost::default();
        let result = Evaluator::new(host).with_stack_budget(0).eval(&program, &Environment::new());
        assert_eq!(result, Err(EvalError::StackOverflow));
    }

    #[test]
    fn closures_share_their_code_with_the_ast() {
        let made = value("let make = fn() { fn(y) { y } }; [make(), make()]");
        let Object::Array(items) = made else { panic!("expected an array") };
        let [Object::Function(a), Object::Function(b)] = &*items else {
            panic!("expected two functions");
        };
        assert!(Rc::ptr_eq(&a.body, &b.body), "each call re-copied the function body");
        assert!(Rc::ptr_eq(&a.params, &b.params));
    }

    #[test]
    fn reading_a_variable_shares_its_array() {
        // Reading `a` must not copy the elements: that would make `a[i]` O(n).
        let env = Environment::new();
        let host = &mut TestHost::default();
        drop(Evaluator::new(host).eval(&parse("let a = [1, 2, 3];"), &env));

        let (Some(Object::Array(first)), Some(Object::Array(second))) =
            (env.get("a"), env.get("a"))
        else {
            panic!("a should be an array");
        };
        assert!(Rc::ptr_eq(&first, &second));
    }

    #[test]
    fn len_counts_bytes_like_the_book() {
        assert_value(r#"len("héllo")"#, &int(6));
        assert_value(r#"len("hello")"#, &int(5));
    }

    #[test]
    fn recursion_works() {
        let fib = "let fib = fn(n) { if (n < 2) { n } else { fib(n - 1) + fib(n - 2) } }; fib(15)";
        assert_eq!(value(fib), int(610));
    }

    #[test]
    fn higher_order_functions_work() {
        let map = "
         let map = fn(arr, f) {
             let iter = fn(arr, acc) {
                 if (len(arr) == 0) { acc } else { iter(rest(arr), push(acc, f(first(arr)))) }
             };
             iter(arr, []);
         };
         map([1, 2, 3], fn(x) { x * 2 })";
        assert_eq!(value(map), array([2, 4, 6]));
    }
}
