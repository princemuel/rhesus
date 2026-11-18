use core::error::Error;
use core::fmt;

use crate::token::Token;

#[derive(Debug, PartialEq, Eq)]
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

pub type ParseResult<'a, T> = Result<T, ParseError<'a>>;
