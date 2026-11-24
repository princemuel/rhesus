use alloc::borrow::Cow;
use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::{hint, ptr};

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
/// [`Session::with_stack_budget`][`crate::Session::with_stack_budget`].
pub const DEFAULT_STACK_BUDGET: usize = 1024 * 1024;

/// State shared by everything evaluated in one run: the host, and how much of
/// the native stack the run may use.
pub(crate) struct Ctx<'ctx> {
    host: &'ctx mut dyn Host,
    /// Where the stack was when the run started.
    stack_base: usize,
    stack_budget: usize,
}

impl<'ctx> Ctx<'ctx> {
    pub(crate) fn new(host: &'ctx mut dyn Host, stack_budget: usize) -> Self {
        Self { host, stack_base: stack_address(), stack_budget }
    }

    fn check_stack<'a>(&self) -> Flow<'a, ()> {
        // it doesn't matter which way the stack grows.
        if stack_address().abs_diff(self.stack_base) > self.stack_budget {
            return Err(EvalError::StackOverflow.into());
        }
        Ok(())
    }
}

/// An address inside the current stack frame: a stand-in for the stack pointer.
///
/// `black_box` is essential. Without it the compiler sees a local that is never
/// read or written, and is free to drop it or pick any address for it, for
/// example a constant. That would make every measurement come out the same, so
/// the budget would never trip and runaway recursion would crash after all.
#[inline(never)]
fn stack_address() -> usize {
    let marker = 0u8;
    hint::black_box(ptr::from_ref(&marker)).addr()
}

/// Why evaluation lhs the normal path. `?` propagates both kinds.
pub(crate) enum Signal<'a> {
    Return(Object<'a>),
    Error(EvalError<'a>),
}

impl<'a> From<EvalError<'a>> for Signal<'a> {
    fn from(err: EvalError<'a>) -> Self { Self::Error(err) }
}

pub(crate) type Flow<'a, T = Object<'a>> = Result<T, Signal<'a>>;

/// Run a whole program. `Ok(None)` means the last statement produced no value
/// (a `let`, or an empty program), so the REPL prints nothing.
///
/// # Errors
///
/// Returns the first runtime error. A top-level `return` just ends the program
/// with its value.
pub fn eval_program<'a>(
    program: &Block<'a>,
    env: &Environment<'a>,
    host: &mut dyn Host,
) -> Result<Option<Object<'a>>, EvalError<'a>> {
    eval_program_with_budget(program, env, host, DEFAULT_STACK_BUDGET)
}
/// [`eval_program`] with an explicit native-stack budget in bytes.
///
/// # Errors
///
/// As [`eval_program`]; also [`EvalError::StackOverflow`] if evaluation needs
/// more than `stack_budget` bytes of stack.
pub fn eval_program_with_budget<'a>(
    program: &Block<'a>,
    env: &Environment<'a>,
    host: &mut dyn Host,
    stack_budget: usize,
) -> Result<Option<Object<'a>>, EvalError<'a>> {
    match eval_block(program, env, &mut Ctx::new(host, stack_budget)) {
        Ok(value) => Ok(value),
        Err(Signal::Return(value)) => Ok(Some(value)),
        Err(Signal::Error(err)) => Err(err),
    }
}

/// Blocks don't open a new scope. Only function calls do.
fn eval_block<'a>(
    block: &Block<'a>,
    env: &Environment<'a>,
    ctx: &mut Ctx<'_>,
) -> Flow<'a, Option<Object<'a>>> {
    block.0.iter().try_fold(None, |_, stmt| eval_stmt(stmt, env, ctx))
}

fn eval_stmt<'a>(
    stmt: &Stmt<'a>,
    env: &Environment<'a>,
    ctx: &mut Ctx<'_>,
) -> Flow<'a, Option<Object<'a>>> {
    match stmt {
        Stmt::Expr(expr) => eval_expr(expr, env, ctx).map(Some),
        Stmt::Return(expr) => Err(Signal::Return(eval_expr(expr, env, ctx)?)),
        Stmt::Let { name, value } => {
            let value = eval_expr(value, env, ctx)?;
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
pub(crate) fn eval_quote<'a>(
    args: &[Expr<'a>],
    env: &Environment<'a>,
    ctx: &mut Ctx<'_>,
) -> Flow<'a> {
    let [arg] = args else {
        return Err(EvalError::WrongArgumentCount { expected: 1, got: args.len() }.into());
    };

    let node = eval_unquote_calls(arg, env, ctx)?;
    Ok(Object::Quote(Rc::new(node)))
}

fn eval_unquote_calls<'a>(
    expr: &Expr<'a>,
    env: &Environment<'a>,
    ctx: &mut Ctx<'_>,
) -> Flow<'a, Expr<'a>> {
    expr.clone().try_modify(&mut |node| -> Flow<'a, Expr<'a>> {
        match node {
            Expr::Call { function, args } if is_ident(&function, "unquote") => {
                let [arg] = <[Expr<'a>; 1]>::try_from(args).map_err(|args| {
                    EvalError::WrongArgumentCount { expected: 1, got: args.len() }
                })?;
                Ok(convert_obj_to_expr(eval_expr(&arg, env, ctx)?)?)
            }
            expr => Ok(expr),
        }
    })
}

pub(crate) fn eval_expr<'a>(expr: &Expr<'a>, env: &Environment<'a>, ctx: &mut Ctx<'_>) -> Flow<'a> {
    ctx.check_stack()?; // it's important to call this here, early in the fn

    match expr {
        Expr::Int(n) => Ok(Object::Integer(*n)),
        Expr::Boolean(b) => Ok(Object::Boolean(*b)),
        Expr::String(s) => Ok(Object::String(s.clone())),
        // Variables shadow builtins
        Expr::Ident(name) => env
            .get(name)
            .or_else(|| builtins::lookup(name))
            .ok_or_else(|| Signal::from(EvalError::IdentifierNotFound(name))),
        Expr::Array(items) => Ok(eval_list(items, env, ctx)?.into()),
        Expr::Hash(pairs) => eval_hash(pairs, env, ctx),
        Expr::Index { lhs, index } => {
            let lhs = eval_expr(lhs, env, ctx)?;
            let index = eval_expr(index, env, ctx)?;
            Ok(eval_index(&lhs, &index)?)
        }
        Expr::Prefix { op, rhs } => {
            let rhs = eval_expr(rhs, env, ctx)?;
            Ok(eval_prefix(*op, &rhs)?)
        }
        Expr::Infix { lhs, op, rhs } => {
            let lhs = eval_expr(lhs, env, ctx)?;
            let rhs = eval_expr(rhs, env, ctx)?;
            Ok(eval_infix(*op, &lhs, &rhs)?)
        }
        Expr::If { condition, consequence, alternative } => {
            let branch = if eval_expr(condition, env, ctx)?.is_truthy() {
                Some(consequence)
            } else {
                alternative.as_ref()
            };

            match branch {
                Some(block) => Ok(eval_block(block, env, ctx)?.unwrap_or_default()),
                None => Ok(Object::Null),
            }
        }
        Expr::Function { params, body } => Ok(Object::Function(closure(params, body, env))),
        Expr::Macro { params, body } => Ok(Object::Macro(closure(params, body, env))),
        // `quote` is a special form: its argument must NOT be evaluated first.
        Expr::Call { function, args } if is_ident(function, "quote") => eval_quote(args, env, ctx),
        Expr::Call { function, args } => {
            let callee = eval_expr(function, env, ctx)?;
            let args = eval_list(args, env, ctx)?;
            apply_function(&callee, args, ctx)
        }
    }
}

fn eval_list<'a>(
    exprs: &[Expr<'a>],
    env: &Environment<'a>,
    ctx: &mut Ctx<'_>,
) -> Flow<'a, Vec<Object<'a>>> {
    exprs.iter().map(|expr| eval_expr(expr, env, ctx)).collect()
}

fn eval_hash<'a>(
    pairs: &[(Expr<'a>, Expr<'a>)],
    env: &Environment<'a>,
    ctx: &mut Ctx<'_>,
) -> Flow<'a> {
    let map = pairs
        .iter()
        .map(|(key, value)| {
            let key = HashKey::try_from(&eval_expr(key, env, ctx)?)?;
            Ok((key, eval_expr(value, env, ctx)?))
        })
        .collect::<Flow<'a, Map<_, _>>>()?;

    Ok(map.into())
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

fn apply_function<'a>(callee: &Object<'a>, args: Vec<Object<'a>>, ctx: &mut Ctx<'_>) -> Flow<'a> {
    match callee {
        Object::Function(func) => {
            let scope = bind_params(func, args)?;
            Ok(run_body(func, &scope, ctx)?)
        }
        Object::Builtin(builtin) => Ok(builtin(&args, ctx.host)?),
        other => Err(EvalError::NotAFunction(other.type_name()).into()),
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
    Rc::new(Function { params: Rc::clone(params), body: Rc::clone(body), env: env.clone() })
}

pub(crate) fn is_ident(expr: &Expr<'_>, name: &str) -> bool {
    matches!(expr, Expr::Ident(ident) if *ident == name)
}

/// A fresh scope, enclosed by the function's own, with each parameter bound.
pub(crate) fn bind_params<'a>(
    func: &Function<'a>,
    args: Vec<Object<'a>>,
) -> Result<Environment<'a>, EvalError<'a>> {
    if func.params.len() != args.len() {
        return Err(EvalError::WrongArgumentCount { expected: func.params.len(), got: args.len() });
    }

    let scope = Environment::new_enclosed(&func.env);
    for (name, arg) in func.params.iter().zip(args) {
        scope.set(name, arg);
    }

    Ok(scope)
}

/// Runs a body in `scope`. A `return` ends the call here and goes no further.
pub(crate) fn run_body<'a>(
    func: &Function<'a>,
    scope: &Environment<'a>,
    ctx: &mut Ctx<'_>,
) -> Result<Object<'a>, EvalError<'a>> {
    match eval_block(&func.body, scope, ctx) {
        Ok(value) => Ok(value.unwrap_or_default()),
        Err(Signal::Return(value)) => Ok(value),
        Err(Signal::Error(err)) => Err(err),
    }
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
        eval_program(&parse(input), &Environment::new(), &mut WriterHost(core::io::sink()))
    }

    fn eval_with<'a>(
        input: &'a str,
        host: &mut dyn Host,
    ) -> Result<Option<Object<'a>>, EvalError<'a>> {
        eval_program(&parse(input), &Environment::new(), host)
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
        let mut run = |src| eval_program(&parse(src), &env, host);

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

        use std::thread;

        const STACK: usize = 64 * 1024 * 1024;
        let src = "let count = fn(n) { if (n == 0) { 0 } else { 1 + count(n - 1) } }; count(1000)";

        // Too deep for the default budget...
        assert_error(src, "stack overflow: recursion too deep");

        // ...but fine with a budget that matches a stack this size.
        let result = thread::Builder::new()
            .stack_size(STACK)
            .spawn(move || {
                let program = parse(src);
                let host = &mut TestHost::default();
                eval_program_with_budget(&program, &Environment::new(), host, STACK / 2)
                    .map(|value| value.map(|v| v.to_string()))
                    .map_err(|err| err.to_string())
            })
            .expect("spawn")
            .join()
            .expect("the evaluator must not overflow the stack");
        assert_eq!(result, Ok(Some("1000".to_string())));
    }

    /// Calls itself `levels` times, with a frame at least 512 bytes big, and
    /// returns the stack address seen at the bottom.
    #[inline(never)]
    fn address_at_depth(levels: usize) -> usize {
        let padding = [0u8; 512];
        let address = if levels == 0 { stack_address() } else { address_at_depth(levels - 1) };
        let _unused = hint::black_box(&padding); // keeps this frame alive across the call
        address
    }

    #[test]
    fn the_stack_address_really_moves_as_frames_pile_up() {
        // If the compiler ever turns `stack_address` into a constant, the stack
        // budget silently stops working. This test fails first.
        let top = stack_address();
        let bottom = address_at_depth(16);
        assert!(
            bottom.abs_diff(top) >= 16 * 512,
            "stack_address barely moved: {top:#x} vs {bottom:#x}"
        );
    }

    #[test]
    fn a_tiny_budget_fails_fast() {
        let program = parse("let f = fn(x) { x }; f(1)");
        let host = &mut TestHost::default();
        let result = eval_program_with_budget(&program, &Environment::new(), host, 0);
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
        drop(eval_program(&parse("let a = [1, 2, 3];"), &env, host));

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
