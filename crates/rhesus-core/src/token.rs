use alloc::borrow::Cow;
use core::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token<'a> {
    Illegal(char),
    /// A string literal that never closed: missing `"`, or a trailing `\`.
    Unterminated,
    Eof,

    // Identifiers + literals
    Ident(&'a str),
    Int(&'a str),
    String(Cow<'a, str>),

    // Operators
    Assign,
    Plus,
    Minus,
    Bang,
    Asterisk,
    Slash,

    Lt,
    Gt,
    Eq,
    NotEq,

    // Delimiters
    Comma,
    Semicolon,
    Colon,

    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,

    // Keywords
    Function,
    Let,
    True,
    False,
    If,
    Else,
    Return,
    Macro,
}

impl Token<'_> {
    #[must_use]
    #[rustfmt::skip]
    pub(crate) const fn lexeme(&self) -> Option<&'static str> {
        Some(match self {
            Self::Assign => "=", Self::Bang => "!",
            Self::Plus => "+", Self::Minus => "-", Self::Asterisk => "*", Self::Slash => "/",
            Self::Lt => "<", Self::Gt => ">", Self::Eq => "==", Self::NotEq => "!=",
            Self::Comma => ",", Self::Semicolon => ";", Self::Colon => ":",
            Self::LParen => "(", Self::RParen => ")",
            Self::LBrace => "{", Self::RBrace => "}",
            Self::LBracket => "[", Self::RBracket => "]",
            Self::If => "if", Self::Else => "else",
            Self::Function => "fn", Self::Macro => "macro",
            Self::Let => "let", Self::Return => "return",
            Self::True => "true", Self::False => "false",
            Self::Illegal(_) | Self::Unterminated | Self::Eof | Self::Ident(_) | Self::Int(_) | Self::String(_) => return None,
        })
    }
}

impl fmt::Display for Token<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Illegal(ch) => write!(f, "ILLEGAL({ch})"),
            Self::Unterminated => f.write_str("unterminated string"),
            Self::Eof => f.write_str("EOF"),
            Self::Ident(s) | Self::Int(s) => f.write_str(s),
            Self::String(s) => write!(f, "\"{s}\""),
            _ => f.write_str(self.lexeme().unwrap_or_default()),
        }
    }
}
