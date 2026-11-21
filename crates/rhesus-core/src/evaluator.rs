use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;

use crate::ast::{Block, Expr, InfixOp as Op, PrefixOp, Stmt};
use crate::builtins;
use crate::environment::Environment;
use crate::errors::EvalError;
use crate::host::Host;
use crate::object::{Function, HashKey, Object};

/// Why evaluation left the normal path. `?` propagates both kinds.
enum Signal<'a> {
    Return(Object<'a>),
    Error(EvalError<'a>),
}

impl<'a> From<EvalError<'a>> for Signal<'a> {
    fn from(err: EvalError<'a>) -> Self { Self::Error(err) }
}

type Flow<'a, T = Object<'a>> = Result<T, Signal<'a>>;

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
    match eval_block(program, env, host) {
        Ok(value) => Ok(value),
        Err(Signal::Return(value)) => Ok(Some(value)),
        Err(Signal::Error(err)) => Err(err),
    }
}

/// Blocks don't open a new scope. Only function calls do.
fn eval_block<'a>(
    block: &Block<'a>,
    env: &Environment<'a>,
    host: &mut dyn Host,
) -> Flow<'a, Option<Object<'a>>> {
    block.0.iter().try_fold(None, |_, stmt| eval_stmt(stmt, env, host))
}

fn eval_stmt<'a>(
    stmt: &Stmt<'a>,
    env: &Environment<'a>,
    host: &mut dyn Host,
) -> Flow<'a, Option<Object<'a>>> {
    match stmt {
        Stmt::Expr(expr) => eval_expr(expr, env, host).map(Some),
        Stmt::Return(expr) => Err(Signal::Return(eval_expr(expr, env, host)?)),
        Stmt::Let { name, value } => {
            let value = eval_expr(value, env, host)?;
            env.set(name, value);
            Ok(None)
        }
    }
}

fn eval_expr<'a>(expr: &Expr<'a>, env: &Environment<'a>, host: &mut dyn Host) -> Flow<'a> {
    match expr {
        Expr::Int(n) => Ok(Object::Integer(*n)),
        Expr::Boolean(b) => Ok(Object::Boolean(*b)),
        Expr::String(s) => Ok(Object::String(s.clone())),
        // Variables shadow builtins, as in the book.
        Expr::Ident(name) => env
            .get(name)
            .or_else(|| builtins::lookup(name))
            .ok_or_else(|| Signal::from(EvalError::IdentifierNotFound(name))),
        Expr::Array(items) => Ok(Object::Array(eval_all(items, env, host)?)),
        Expr::Hash(pairs) => eval_hash(pairs, env, host),
        Expr::Index { lhs: left, index } => {
            let left = eval_expr(left, env, host)?;
            let index = eval_expr(index, env, host)?;
            Ok(eval_index(&left, &index)?)
        }
        Expr::Prefix { op, rhs: right } => {
            let right = eval_expr(right, env, host)?;
            Ok(eval_prefix(*op, &right)?)
        }
        Expr::Infix { lhs: left, op, rhs: right } => {
            let left = eval_expr(left, env, host)?;
            let right = eval_expr(right, env, host)?;
            Ok(eval_infix(*op, &left, &right)?)
        }
        Expr::If { condition, consequence, alternative } => {
            let branch = if eval_expr(condition, env, host)?.is_truthy() {
                Some(consequence)
            } else {
                alternative.as_ref()
            };
            match branch {
                Some(block) => Ok(eval_block(block, env, host)?.unwrap_or(Object::Null)),
                None => Ok(Object::Null),
            }
        }
        Expr::Function { params, body } => Ok(Object::Function(Rc::new(Function {
            params: params.clone(),
            body: body.clone(),
            env: env.clone(),
        }))),
        Expr::Call { function, args } => {
            let callee = eval_expr(function, env, host)?;
            let args = eval_all(args, env, host)?;
            apply_function(&callee, args, host)
        }
    }
}

fn eval_all<'a>(
    exprs: &[Expr<'a>],
    env: &Environment<'a>,
    host: &mut dyn Host,
) -> Flow<'a, Vec<Object<'a>>> {
    exprs.iter().map(|expr| eval_expr(expr, env, host)).collect()
}

/// Later duplicate keys overwrite earlier ones, as with Go's map.
fn eval_hash<'a>(
    pairs: &[(Expr<'a>, Expr<'a>)],
    env: &Environment<'a>,
    host: &mut dyn Host,
) -> Flow<'a> {
    let map = pairs
        .iter()
        .map(|(key, value)| -> Flow<'a, (HashKey<'a>, Object<'a>)> {
            let key = HashKey::try_from(&eval_expr(key, env, host)?)?;
            Ok((key, eval_expr(value, env, host)?))
        })
        .collect::<Flow<'a, BTreeMap<_, _>>>()?;
    Ok(Object::Hash(map))
}

/// Out-of-range and negative array indexes, and missing hash keys, give `null`.
fn eval_index<'a>(left: &Object<'a>, index: &Object<'a>) -> Result<Object<'a>, EvalError<'a>> {
    match (left, index) {
        (Object::Array(items), Object::Integer(i)) => {
            Ok(usize::try_from(*i).ok().and_then(|i| items.get(i)).cloned().unwrap_or(Object::Null))
        }
        (Object::Hash(map), key) => {
            Ok(map.get(&HashKey::try_from(key)?).cloned().unwrap_or(Object::Null))
        }
        _ => Err(EvalError::IndexNotSupported(left.type_name())),
    }
}

fn apply_function<'a>(callee: &Object<'a>, args: Vec<Object<'a>>, host: &mut dyn Host) -> Flow<'a> {
    match callee {
        Object::Function(func) => {
            if func.params.len() != args.len() {
                return Err(EvalError::WrongArgumentCount {
                    expected: func.params.len(),
                    got: args.len(),
                }
                .into());
            }

            let scope = Environment::new_enclosed(&func.env);
            for (name, arg) in func.params.iter().zip(args) {
                scope.set(name, arg);
            }

            // A `return` ends this call and goes no further.
            match eval_block(&func.body, &scope, host) {
                Ok(value) => Ok(value.unwrap_or(Object::Null)),
                Err(Signal::Return(value)) => Ok(value),
                Err(err) => Err(err),
            }
        }
        Object::Builtin(builtin) => Ok(builtin(&args, host)?),
        other => Err(EvalError::NotAFunction(other.type_name()).into()),
    }
}

fn eval_prefix<'a>(op: PrefixOp, right: &Object<'a>) -> Result<Object<'a>, EvalError<'a>> {
    match (op, right) {
        (PrefixOp::Bang, _) => Ok(Object::Boolean(!right.is_truthy())),
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
        (_, Object::Integer(left), Object::Integer(right)) => eval_int_infix(op, *left, *right),
        (Op::Plus, Object::String(left), Object::String(right)) => {
            Ok(Object::String(Cow::Owned(format!("{left}{right}"))))
        }
        // `==` and `!=` work on any pair, before the error cases
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

    fn str_obj(s: &str) -> Object<'_> { Object::String(Cow::Borrowed(s)) }

    fn array<'a>(items: impl IntoIterator<Item = i64>) -> Object<'a> {
        Object::Array(items.into_iter().map(Object::Integer).collect())
    }

    fn value(input: &str) -> Object<'_> { eval(input).expect("eval error").expect("no value") }

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
        assert_eq!(func.params, ["x"]);
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
        assert_eq!(result, Ok(Some(Object::Array(vec!["a".into(), Object::Null]))));
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
    fn input_replays_lines_in_order_then_null() {
        let mut host = TestHost { input: vec!["a".into(), "b".into()], ..TestHost::default() };
        let result = eval_with("[input(), input(), input()]", &mut host);
        assert_eq!(result, Ok(Some(Object::Array(vec!["a".into(), "b".into(), Object::Null]))));
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
}
