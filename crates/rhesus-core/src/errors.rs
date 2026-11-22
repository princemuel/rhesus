use core::error::Error;
use core::fmt;

use crate::ast::{InfixOp, PrefixOp};
use crate::token::Token;

pub type ParseResult<'a, T> = Result<T, ParseError<'a>>;
pub type EvalResult<'a, T> = Result<T, EvalError<'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError<'a> {
    UnexpectedToken { expected: &'static str, got: Token<'a> },
    UnexpectedEof { expected: &'static str },
    InvalidInt(&'a str),
}
impl ParseError<'_> {
    #[must_use]
    pub const fn ended_at_semicolon(&self) -> bool {
        matches!(self, Self::UnexpectedToken { got: Token::Semicolon, .. })
    }
}
impl fmt::Display for ParseError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedToken { expected, got } => {
                write!(f, "expected {expected}, got {got}")
            }
            Self::UnexpectedEof { expected } => {
                write!(f, "expected {expected}, got end of input")
            }
            Self::InvalidInt(raw) => write!(f, "integer literal out of range: {raw}"),
        }
    }
}
impl Error for ParseError<'_> {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvalError<'a> {
    TypeMismatch {
        lhs: &'static str,
        op: InfixOp,
        rhs: &'static str,
    },
    UnknownInfixOperator {
        lhs: &'static str,
        op: InfixOp,
        rhs: &'static str,
    },
    UnknownPrefixOperator {
        op: PrefixOp,
        operand: &'static str,
    },
    IdentifierNotFound(&'a str),
    NotAFunction(&'static str),
    WrongArgumentCount {
        expected: usize,
        got: usize,
    },
    ArgumentNotSupported {
        builtin: &'static str,
        got: &'static str,
    },
    ArgumentMustBe {
        builtin: &'static str,
        expected: &'static str,
        got: &'static str,
    },
    UnusableAsHashKey(&'static str),
    IndexNotSupported(&'static str),
    DivisionByZero,
    /// `unquote` produces a value that has no source-code form (e.g. a
    /// function)
    CannotUnquote(&'static str),
    /// A macro body must evaluate to a `quote(...)`, the code to splice in.
    MacroMustReturnQuote(&'static str),
    InvalidDigit,
    IntegerOverflow,
    Host(HostError),
}

impl fmt::Display for EvalError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TypeMismatch { lhs, op, rhs } => {
                write!(f, "type mismatch: {lhs} {op} {rhs}")
            }
            Self::UnknownInfixOperator { lhs, op, rhs } => {
                write!(f, "unknown operator: {lhs} {op} {rhs}")
            }
            Self::UnknownPrefixOperator { op, operand } => {
                write!(f, "unknown operator: {op}{operand}")
            }
            Self::IdentifierNotFound(name) => write!(f, "identifier not found: {name}"),
            Self::NotAFunction(ty) => write!(f, "not a function: {ty}"),
            Self::WrongArgumentCount { expected, got } => {
                write!(f, "wrong number of arguments. got={got}, want={expected}")
            }
            Self::ArgumentNotSupported { builtin, got } => {
                write!(f, "argument to `{builtin}` not supported, got {got}")
            }
            Self::ArgumentMustBe { builtin, expected, got } => {
                write!(f, "argument to `{builtin}` must be {expected}, got {got}")
            }
            Self::UnusableAsHashKey(got) => write!(f, "unusable as hash key: {got}"),
            Self::IndexNotSupported(got) => write!(f, "index operator not supported: {got}"),
            Self::CannotUnquote(got) => write!(f, "cannot unquote a value of type {got}"),
            Self::MacroMustReturnQuote(got) => write!(f, "macro must return a QUOTE, got {got}"),
            Self::DivisionByZero => f.write_str("division by zero"),
            Self::IntegerOverflow => f.write_str("integer overflow"),
            Self::InvalidDigit => f.write_str("invalid digit"),
            Self::Host(err) => write!(f, "{err}"),
        }
    }
}

impl Error for EvalError<'_> {}

impl From<HostError> for EvalError<'_> {
    fn from(err: HostError) -> Self { Self::Host(err) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostError {
    /// The host doesn't provide this capability.
    Unsupported,
    /// The capability exists but the operation failed.
    Io,
}
impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("operation not supported by host"),
            Self::Io => f.write_str("host I/O error"),
        }
    }
}
impl Error for HostError {}
