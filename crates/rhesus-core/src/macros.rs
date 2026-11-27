//! `quote`/`unquote` and macro expansion.
//!
//! Macros run in their own pass *before* evaluation

use alloc::rc::Rc;
use alloc::vec::Vec;

use crate::ast::{Block, Expr, Stmt};
use crate::environment::Environment;
use crate::errors::EvalError;
use crate::evaluator::{DEFAULT_STACK_BUDGET, Evaluator, bind_params_to_env};
use crate::host::Host;
use crate::object::{Function, Object};

/// Removes top-level macro definitions from `program` and stores them in `env`.
/// Also returns how many were defined, so callers can skip expansion if none
/// exist.
#[must_use]
pub fn define_macros<'a>(program: Block<'a>, env: &Environment<'a>) -> (Block<'a>, usize) {
    let mut rest = Vec::new();
    let mut defined = 0;

    for stmt in program.0 {
        if let Stmt::Let { name, value: Expr::Macro { params, body } } = stmt {
            env.set(
                name,
                Object::Macro(Rc::new(Function {
                    name: Some(name),
                    params,
                    body,
                    env: Environment::viewing(env),
                })),
            );

            defined += 1;
        } else {
            rest.push(stmt);
        }
    }
    (Block(rest), defined)
}

/// Replaces every call to a macro bound in `env` with the AST node it returns.
///
/// Expansion runs as a single bottom-up pass using [`DEFAULT_STACK_BUDGET`].
/// The code returned by a macro is not expanded again, so a macro returning a
/// call to another macro leaves that inner call untouched.
///
/// See [`expand_macros_with_budget`] to specify a custom stack limit.
///
/// # Errors
///
/// Returns an [`EvalError`] if a macro is called with the wrong number of
/// arguments, encounters a runtime error or [`EvalError::StackOverflow`] while
/// executing, or returns anything other than a `quote(..)`.
pub fn expand_macros<'a>(
    program: Block<'a>,
    env: &Environment<'a>,
    host: &mut dyn Host,
) -> Result<Block<'a>, EvalError<'a>> {
    expand_macros_with_budget(program, env, host, DEFAULT_STACK_BUDGET)
}

/// Replaces every call to a macro bound in `env` with the AST node it returns,
/// enforcing an explicit native-stack budget in bytes.
///
/// Behaves identically to [`expand_macros`], except macro execution is capped
/// at `stack_budget` bytes of call stack (similar to
/// [`Evaluator::with_stack_budget`]).
///
/// # Errors
///
/// Returns an [`EvalError`] under the same conditions as [`expand_macros`],
/// with [`EvalError::StackOverflow`] triggered if macro execution exceeds
/// `stack_budget`.
pub fn expand_macros_with_budget<'a>(
    program: Block<'a>,
    env: &Environment<'a>,
    host: &mut dyn Host,
    stack_budget: usize,
) -> Result<Block<'a>, EvalError<'a>> {
    let mut evaluator = Evaluator::new(host).with_stack_budget(stack_budget);
    program.try_modify(&mut |expr| expand_macro_call(expr, env, &mut evaluator))
}

/// Expands `expr` if it is a call to a macro bound in `env`; otherwise returns
/// `expr` unchanged.
///
/// Arguments are passed to the macro body as unevaluated [`Object::Quote`]
/// nodes rather than evaluated values, and the macro must return a quote to
/// splice back into the AST.
fn expand_macro_call<'a>(
    expr: Expr<'a>,
    env: &Environment<'a>,
    evaluator: &mut Evaluator<'_>,
) -> Result<Expr<'a>, EvalError<'a>> {
    let Expr::Call { function, args } = &expr else {
        return Ok(expr);
    };
    let Expr::Ident(name) = **function else {
        return Ok(expr);
    };
    let Some(Object::Macro(mac)) = env.get(name) else {
        return Ok(expr);
    };

    // Macro arguments are passed as unevaluated code, never as values.
    let quoted_args = args.iter().map(|arg| Object::Quote(Rc::new(arg.clone()))).collect();
    let scope = bind_params_to_env(&mac, quoted_args)?;

    match evaluator.eval_func_or_macro_body(&mac, &scope)? {
        Object::Quote(node) => Ok(Rc::unwrap_or_clone(node)),
        other => Err(EvalError::MacroMustReturnQuote(other.type_name())),
    }
}

/// Turns an evaluated value back into source code, so it can be spliced into a
/// quote.
pub(crate) fn convert_obj_to_expr(object: Object<'_>) -> Result<Expr<'_>, EvalError<'_>> {
    match object {
        Object::Integer(n) => Ok(Expr::Int(n)),
        Object::Boolean(b) => Ok(Expr::Boolean(b)),
        Object::String(s) => Ok(Expr::String(s)),
        Object::Quote(node) => Ok(Rc::unwrap_or_clone(node)),
        Object::Array(items) => items
            .iter()
            .cloned()
            .map(convert_obj_to_expr)
            .collect::<Result<_, _>>()
            .map(Expr::Array),
        Object::Hash(map) => Rc::unwrap_or_clone(map)
            .into_iter()
            .map(|(key, value)| Ok((convert_obj_to_expr(key.into())?, convert_obj_to_expr(value)?)))
            .collect::<Result<_, _>>()
            .map(Expr::Hash),
        other => Err(EvalError::CannotUnquote(other.type_name())),
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString as _};

    use super::*;
    use crate::host::TestHost;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn parse(src: &str) -> Block<'_> {
        Parser::new(Lexer::new(src))
            .parse_program()
            .unwrap_or_else(|errs| panic!("parse errors for {src:?}: {errs:#?}"))
    }

    fn eval(input: &str) -> Result<Option<Object<'_>>, EvalError<'_>> {
        Evaluator::new(&mut TestHost::default()).eval(&parse(input), &Environment::new())
    }

    /// Evaluates `input` and returns the source text of the quote it produced.
    fn quoted(input: &str) -> String {
        match eval(input).expect("eval error").expect("no value") {
            Object::Quote(node) => node.to_string(),
            other => panic!("expected a QUOTE for {input}, got {other}"),
        }
    }

    fn expand(input: &str) -> Block<'_> {
        let env = Environment::new();
        let (program, _) = define_macros(parse(input), &env);
        expand_macros(program, &env, &mut TestHost::default()).expect("expand error")
    }

    fn expand_error(input: &str) -> String {
        let env = Environment::new();
        let (program, _) = define_macros(parse(input), &env);
        expand_macros(program, &env, &mut TestHost::default())
            .expect_err("expected an expansion error")
            .to_string()
    }

    fn assert_eval_error(input: &str, want: &str) {
        let err = eval(input).expect_err("expected an error");
        assert_eq!(err.to_string(), want, "input: {input}");
    }

    /// The whole pipeline, as the REPL runs it: define, expand, evaluate.
    fn run_with<'a>(input: &'a str, host: &mut TestHost) -> Option<Object<'a>> {
        let env = Environment::new();
        let macro_env = Environment::new();
        let (program, _) = define_macros(parse(input), &macro_env);
        let program = expand_macros(program, &macro_env, host).expect("expand error");
        Evaluator::new(host).eval(&program, &env).expect("eval error")
    }

    #[test]
    fn quote() {
        let tests = [
            ("quote(5)", "5"),
            ("quote(5 + 8)", "(+ 5 8)"),
            ("quote(foobar)", "foobar"),
            ("quote(foobar + barfoo)", "(+ foobar barfoo)"),
        ];
        for (input, want) in tests {
            assert_eq!(quoted(input), want, "input: {input}");
        }
    }

    #[test]
    fn quote_unquote() {
        let tests = [
            ("quote(unquote(4))", "4"),
            ("quote(unquote(4 + 4))", "8"),
            ("quote(8 + unquote(4 + 4))", "(+ 8 8)"),
            ("quote(unquote(4 + 4) + 8)", "(+ 8 8)"),
            ("let foobar = 8; quote(foobar)", "foobar"),
            ("let foobar = 8; quote(unquote(foobar))", "8"),
            ("quote(unquote(true))", "true"),
            ("quote(unquote(true == false))", "false"),
            ("quote(unquote(quote(4 + 4)))", "(+ 4 4)"),
            (
                "let quotedInfixExpression = quote(4 + 4);
             quote(unquote(4 + 4) + unquote(quotedInfixExpression))",
                "(+ 8 (+ 4 4))",
            ),
        ];
        for (input, want) in tests {
            assert_eq!(quoted(input), want, "input: {input}");
        }
    }

    #[test]
    fn define_macros_stores_only_macros_and_removes_them() {
        let input = "
    let number = 1;
    let function = fn(x, y) { x + y };
    let mymacro = macro(x, y) { x + y; };";

        let env = Environment::new();
        let (program, defined) = define_macros(parse(input), &env);

        assert_eq!(defined, 1, "exactly one macro was defined");
        assert_eq!(program.0.len(), 2, "the macro definition should be removed");
        assert!(env.get("number").is_none(), "number is not a macro");
        assert!(env.get("function").is_none(), "function is not a macro");

        let Some(Object::Macro(mac)) = env.get("mymacro") else {
            panic!("mymacro was not defined as a macro");
        };
        assert_eq!(*mac.params, ["x", "y"]);
        assert_eq!(mac.body.to_string(), "(do (+ x y))");
    }

    #[test]
    fn expand_macros_replaces_calls_with_their_expansion() {
        let tests = [
            (
                "let infixExpression = macro() { quote(1 + 2); };
             infixExpression();",
                "(1 + 2)",
            ),
            (
                "let reverse = macro(a, b) { quote(unquote(b) - unquote(a)); };
             reverse(2 + 2, 10 - 5);",
                "(10 - 5) - (2 + 2)",
            ),
            (
                r#"
            let unless = macro(condition, consequence, alternative) {
                quote(if (!(unquote(condition))) {
                    unquote(consequence);
                } else {
                    unquote(alternative);
                });
            };

            unless(10 > 5, puts("not greater"), puts("greater"));"#,
                r#"if (!(10 > 5)) { puts("not greater") } else { puts("greater") }"#,
            ),
        ];
        for (input, expected) in tests {
            assert_eq!(expand(input), parse(expected), "input: {input}");
        }
    }

    #[test]
    fn quote_does_not_evaluate_its_argument() {
        // Evaluating 1 / 0 would be an error; quoting it must not be.
        assert_eq!(quoted("quote(1 / 0)"), "(/ 1 0)");
    }

    #[test]
    fn quote_object_prints_as_quote() {
        let value = eval("quote(1 + 2)").unwrap().unwrap();
        assert_eq!(value.to_string(), "QUOTE((+ 1 2))");
        assert_eq!(value.type_name(), "QUOTE");
    }

    #[test]
    fn unquote_reaches_inside_calls_and_collections() {
        assert_eq!(quoted("quote(f(unquote(1 + 2)))"), "(call f 3)");
        assert_eq!(quoted("quote([unquote(1 + 1), 3])"), "(array 2 3)");
        assert_eq!(quoted(r#"quote({"a": unquote(1 + 1)})"#), r#"(hash ("a" 2))"#);
    }

    #[test]
    fn unquote_splices_strings_arrays_and_hashes() {
        assert_eq!(quoted(r#"quote(unquote("hi"))"#), r#""hi""#);
        assert_eq!(quoted("quote(unquote([1, 2]))"), "(array 1 2)");
        assert_eq!(quoted(r#"quote(unquote({"a": 1}))"#), r#"(hash ("a" 1))"#);
        assert_eq!(quoted("quote(unquote(-5))"), "-5");
    }

    #[test]
    fn unquote_errors() {
        let tests = [
            ("quote(unquote(fn(x) { x }))", "cannot unquote a value of type FUNCTION"),
            ("quote(unquote(if (false) { 1 }))", "cannot unquote a value of type NULL"),
            ("quote(unquote(foobar))", "identifier not found: foobar"),
            ("quote(unquote())", "wrong number of arguments. got=0, want=1"),
            ("quote(unquote(1, 2))", "wrong number of arguments. got=2, want=1"),
        ];
        for (input, want) in tests {
            assert_eval_error(input, want);
        }
    }

    #[test]
    fn quote_arity_is_checked() {
        assert_eval_error("quote()", "wrong number of arguments. got=0, want=1");
        assert_eval_error("quote(1, 2)", "wrong number of arguments. got=2, want=1");
    }

    #[test]
    fn macro_arguments_are_not_evaluated() {
        // If the argument were evaluated first, 1 / 0 would fail.
        let expanded = expand("let m = macro(x) { quote(1) }; m(1 / 0)");
        assert_eq!(expanded, parse("1"));
    }

    #[test]
    fn macro_calls_nested_in_call_arguments_are_expanded() {
        // Another place the book's `Modify` stops short of.
        let expanded = expand(
            "let identity = fn(x) { x };
         let double = macro(x) { quote(unquote(x) * 2) };
         identity(double(21))",
        );
        assert_eq!(expanded, parse("let identity = fn(x) { x }; identity(21 * 2)"));
    }

    #[test]
    fn expansion_is_a_single_pass() {
        let expanded = expand(
            "let a = macro() { quote(1) };
         let b = macro() { quote(a()) };
         b()",
        );
        assert_eq!(expanded, parse("a()"), "the returned call to `a` is not expanded again");
    }

    #[test]
    fn macros_cannot_see_runtime_variables() {
        // Expansion happens before anything runs, so `two` does not exist yet.
        let msg = expand_error(
            "let two = 2;
         let m = macro() { quote(unquote(two)) };
         m()",
        );
        assert_eq!(msg, "identifier not found: two");
    }

    #[test]
    fn macro_expansion_errors() {
        let tests = [
            ("let m = macro() { 1 }; m()", "macro must return a QUOTE, got INTEGER"),
            ("let m = macro() { }; m()", "macro must return a QUOTE, got NULL"),
            ("let m = macro(a) { quote(1) }; m()", "wrong number of arguments. got=0, want=1"),
            ("let m = macro() { quote(1) }; m(1)", "wrong number of arguments. got=1, want=0"),
            ("let m = macro() { foobar }; m()", "identifier not found: foobar"),
        ];
        for (input, want) in tests {
            assert_eq!(expand_error(input), want, "input: {input}");
        }
    }

    #[test]
    fn macro_literals_are_values_but_not_callable_at_runtime() {
        let value = eval("macro(x) { x }").unwrap().unwrap();
        assert_eq!(value.type_name(), "MACRO");
        assert_eq!(value.to_string(), "macro(x) {\n(do x)\n}");

        assert_eval_error("let f = fn() { macro() { quote(1) } }; f()()", "not a function: MACRO");
    }

    #[test]
    fn unless_runs_only_the_branch_it_picks() {
        let input = r#"
        let unless = macro(condition, consequence, alternative) {
            quote(if (!(unquote(condition))) {
                unquote(consequence);
            } else {
                unquote(alternative);
            });
        };
        unless(10 > 5, puts("not greater"), puts("greater"));"#;

        let mut host = TestHost::default();
        let result = run_with(input, &mut host);

        assert_eq!(result, Some(Object::Null), "puts returns null");
        assert_eq!(host.output, ["greater"], "the other branch must never run");
    }

    #[test]
    fn macros_compose_with_functions_end_to_end() {
        let input = "
        let double = macro(x) { quote(unquote(x) * 2) };
        let add = fn(a, b) { a + b };
        add(double(5), double(1 + 1))";
        let result = run_with(input, &mut TestHost::default());
        assert_eq!(result, Some(Object::Integer(14)));
    }
}
