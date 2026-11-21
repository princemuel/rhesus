use alloc::string::ToString as _;

use crate::errors::EvalError;
use crate::host::Host;
use crate::object::{BuiltinFn, BuiltinResult, Object};

/// Find a builtin by name. The evaluator checks the environment first, then
/// this.
#[must_use]
pub fn lookup<'a>(name: &str) -> Option<Object<'a>> {
    let b = |f: BuiltinFn<'a>| Some(Object::Builtin(f));
    match name {
        "len" => b(len),
        "first" => b(first),
        "last" => b(last),
        "rest" => b(rest),
        "push" => b(push),
        "puts" => b(puts),
        "input" => b(input),
        "clock" => b(clock),
        _ => None,
    }
}

/// Check the argument count and hand back a fixed-size array to destructure.
fn expect_args<'x, 'a, const N: usize>(
    args: &'x [Object<'a>],
) -> Result<&'x [Object<'a>; N], EvalError<'a>> {
    args.try_into().map_err(|_err| EvalError::WrongArgumentCount { expected: N, got: args.len() })
}

fn array_arg<'x, 'a>(
    builtin: &'static str,
    arg: &'x Object<'a>,
) -> Result<&'x [Object<'a>], EvalError<'a>> {
    match arg {
        Object::Array(items) => Ok(items),
        other => {
            Err(EvalError::ArgumentMustBe { builtin, expected: "ARRAY", got: other.type_name() })
        }
    }
}

fn len<'a>(args: &[Object<'a>], _host: &mut dyn Host) -> BuiltinResult<'a> {
    let [arg] = expect_args::<1>(args)?;
    let n = match arg {
        Object::String(s) => s.len(),
        Object::Array(items) => items.len(),
        other => {
            return Err(EvalError::ArgumentNotSupported { builtin: "len", got: other.type_name() });
        }
    };
    i64::try_from(n).map(Object::Integer).map_err(|_err| EvalError::IntegerOverflow)
}

fn first<'a>(args: &[Object<'a>], _host: &mut dyn Host) -> BuiltinResult<'a> {
    let [arg] = expect_args::<1>(args)?;
    Ok(array_arg("first", arg)?.first().cloned().into())
}

fn last<'a>(args: &[Object<'a>], _host: &mut dyn Host) -> BuiltinResult<'a> {
    let [arg] = expect_args::<1>(args)?;
    Ok(array_arg("last", arg)?.last().cloned().into())
}

fn rest<'a>(args: &[Object<'a>], _host: &mut dyn Host) -> BuiltinResult<'a> {
    let [arg] = expect_args::<1>(args)?;
    Ok(array_arg("rest", arg)?.split_first().map(|(_, tail)| Object::Array(tail.to_vec())).into())
}

fn push<'a>(args: &[Object<'a>], _host: &mut dyn Host) -> BuiltinResult<'a> {
    let [array, value] = expect_args::<2>(args)?;
    let mut items = array_arg("push", array)?.to_vec();
    items.push(value.clone());
    Ok(Object::Array(items))
}

fn puts<'a>(args: &[Object<'a>], host: &mut dyn Host) -> BuiltinResult<'a> {
    for arg in args {
        host.write_line(&arg.to_string())?;
    }
    Ok(Object::Null)
}

/// `input()` returns the next line as a string, or `null` at end of input.
fn input<'a>(args: &[Object<'a>], host: &mut dyn Host) -> BuiltinResult<'a> {
    let [] = expect_args::<0>(args)?;
    Ok(host.read_line()?.into())
}

/// `clock()` returns milliseconds as an integer.
fn clock<'a>(args: &[Object<'a>], host: &mut dyn Host) -> BuiltinResult<'a> {
    let [] = expect_args::<0>(args)?;
    Ok(Object::Integer(host.now_millis()?))
}
