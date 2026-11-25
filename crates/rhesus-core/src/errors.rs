use alloc::string::String;
use alloc::vec::Vec;
use core::error::Error as StdError;
use core::fmt;

use crate::ast::{InfixOp, PrefixOp};
use crate::token::Token;

#[expect(clippy::error_impl_error)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// One message per syntax error.
    Parse(Vec<String>),
    /// A runtime error (or a macro-expansion error).
    Eval(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(msg) => f.write_str(&msg.join("\n  ")),
            Self::Eval(msg) => f.write_str(msg),
        }
    }
}

impl StdError for Error {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError<'a> {
    UnexpectedToken { expected: &'static str, got: Token<'a> },
    UnexpectedEof { expected: &'static str },
    InvalidInt(&'a str),
    NestingTooDeep,
}
impl ParseError<'_> {
    #[must_use]
    pub const fn is_incomplete(&self) -> bool {
        matches!(
            self,
            Self::UnexpectedEof { .. }
                | Self::UnexpectedToken { got: Token::Illegal('"' | '\\'), .. }
        )
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
            Self::NestingTooDeep => {
                f.write_str("expression too deeply nested (or too many chained operators)")
            }
        }
    }
}
impl StdError for ParseError<'_> {}

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
    StackOverflow,
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
            Self::StackOverflow => f.write_str("stack overflow: recursion too deep"),
            Self::InvalidDigit => f.write_str("invalid digit"),
            Self::Host(err) => write!(f, "{err}"),
        }
    }
}

impl StdError for EvalError<'_> {}

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
impl StdError for HostError {}

/// A malformed or impossible instruction, from encoding or decoding bytecode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpcodeError {
    Unknown(u8),
    OperandCount { opcode: &'static str, expected: usize, got: usize },
    OperandOverflow { opcode: &'static str, operand: usize, bytes: usize },
    Truncated { opcode: &'static str, expected: usize, got: usize },
    InvalidPatchOffset { offset: usize, len: usize },
}

impl fmt::Display for OpcodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(byte) => write!(f, "opcode {byte} undefined"),
            Self::OperandCount { opcode, expected, got } => {
                write!(f, "{opcode} expects {expected} operand(s), got {got}")
            }
            Self::OperandOverflow { opcode, operand, bytes } => {
                write!(f, "operand {operand} overflows {bytes}-byte slot for {opcode}")
            }
            Self::Truncated { opcode, expected, got } => {
                write!(f, "truncated {opcode}: needs {expected} operand byte(s), got {got}")
            }
            Self::InvalidPatchOffset { offset, len } => {
                write!(f, "cannot patch instruction at offset {offset} (bytecode len is {len})")
            }
        }
    }
}
impl StdError for OpcodeError {}
