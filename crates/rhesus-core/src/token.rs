use alloc::borrow::Cow;
use core::fmt;
use core::iter::Peekable;
use core::str::CharIndices;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token<'a> {
    Illegal(char),
    EOF,

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

impl<'a> Token<'a> {
    #[must_use]
    pub const fn lookup_ident(ident: &'a str) -> Self {
        match ident {
            "fn" => Self::Function,
            "let" => Self::Let,
            "true" => Self::True,
            "false" => Self::False,
            "if" => Self::If,
            "else" => Self::Else,
            "return" => Self::Return,
            "macro" => Self::Macro,
            _ => Self::Ident(ident),
        }
    }
}

impl fmt::Display for Token<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Illegal(ch) => write!(f, "ILLEGAL({ch})"),
            Self::EOF => write!(f, "EOF"),

            // Identifiers + literals
            Self::Ident(ident) => write!(f, "{ident}"),
            Self::Int(num) => write!(f, "{num}"),
            Self::String(s) => write!(f, "\"{s}\""),

            // Operators
            Self::Assign => write!(f, "="),
            Self::Plus => write!(f, "+"),
            Self::Minus => write!(f, "-"),
            Self::Bang => write!(f, "!"),
            Self::Asterisk => write!(f, "*"),
            Self::Slash => write!(f, "/"),

            Self::Lt => write!(f, "<"),
            Self::Gt => write!(f, ">"),
            Self::Eq => write!(f, "=="),
            Self::NotEq => write!(f, "!="),

            // Delimiters
            Self::Comma => write!(f, ","),
            Self::Semicolon => write!(f, ";"),
            Self::Colon => write!(f, ":"),

            Self::LParen => write!(f, "("),
            Self::RParen => write!(f, ")"),
            Self::LBrace => write!(f, "{{"),
            Self::RBrace => write!(f, "}}"),
            Self::LBracket => write!(f, "["),
            Self::RBracket => write!(f, "]"),

            // Keywords
            Self::Function => write!(f, "fn"),
            Self::Let => write!(f, "let"),
            Self::True => write!(f, "true"),
            Self::False => write!(f, "false"),
            Self::If => write!(f, "if"),
            Self::Else => write!(f, "else"),
            Self::Return => write!(f, "return"),
            Self::Macro => write!(f, "macro"),
        }
    }
}
