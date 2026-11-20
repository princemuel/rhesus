use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;

use crate::token::Token;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block<'a>(pub Vec<Stmt<'a>>);

impl fmt::Display for Block<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(do")?;
        for stmt in &self.0 {
            write!(f, " {stmt}")?;
        }
        write!(f, ")")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program<'a>(pub Vec<Stmt<'a>>);

impl fmt::Display for Program<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, stmt) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{stmt}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt<'a> {
    Let { name: &'a str, value: Expr<'a> },
    Return(Expr<'a>),
    Expr(Expr<'a>),
}

impl fmt::Display for Stmt<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Let { name, value } => write!(f, "(let {name} {value})"),
            Self::Return(value) => write!(f, "(return {value})"),
            Self::Expr(expr) => write!(f, "{expr}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr<'a> {
    Ident(&'a str),
    Int(i64),
    String(Cow<'a, str>),
    Boolean(bool),
    Prefix { op: PrefixOp, right: Box<Self> },
    Infix { left: Box<Self>, op: InfixOp, right: Box<Self> },
    If { condition: Box<Self>, consequence: Block<'a>, alternative: Option<Block<'a>> },
    Function { params: Vec<&'a str>, body: Block<'a> },
    Call { function: Box<Self>, args: Vec<Self> },
}

impl fmt::Display for Expr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ident(name) => write!(f, "{name}"),
            Self::Int(val) => write!(f, "{val}"),
            Self::String(s) => write!(f, "{s}"),
            Self::Boolean(b) => write!(f, "{b}"),
            Self::Prefix { op, right } => write!(f, "({op} {right})"),
            Self::Infix { left, op, right } => write!(f, "({op} {left} {right})"),
            Self::If { condition, consequence, alternative: None } => {
                write!(f, "(if {condition} {consequence})")
            }
            Self::If { condition, consequence, alternative: Some(alt) } => {
                write!(f, "(if {condition} {consequence} {alt})")
            }
            Self::Function { params, body } => {
                write!(f, "(fn ({}) {body})", params.join(" "))
            }
            Self::Call { function, args } => {
                write!(f, "(call {function}")?;
                for arg in args {
                    write!(f, " {arg}")?;
                }
                write!(f, ")")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefixOp {
    Bang,
    Minus,
}

impl fmt::Display for PrefixOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bang => write!(f, "!"),
            Self::Minus => write!(f, "-"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfixOp {
    Plus,
    Minus,
    Slash,
    Asterisk,
    Eq,
    NotEq,
    Lt,
    Gt,
}

impl fmt::Display for InfixOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plus => write!(f, "+"),
            Self::Minus => write!(f, "-"),
            Self::Slash => write!(f, "/"),
            Self::Asterisk => write!(f, "*"),
            Self::Eq => write!(f, "=="),
            Self::NotEq => write!(f, "!="),
            Self::Lt => write!(f, "<"),
            Self::Gt => write!(f, ">"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Precedence {
    Lowest,
    Equals,      // == !=
    LessGreater, // < >
    Sum,         // + -
    Product,     // * /
    Prefix,      // -x !x
    Call,        // f(x)
}

#[derive(Clone, Copy, Debug)]
pub enum Infix {
    Binary(InfixOp),
    Call,
}

#[must_use]
/// How a token behaves when it appears *after* an expr.
pub const fn infix_rule(token: &Token<'_>) -> Option<(Precedence, Infix)> {
    use Infix::{Binary, Call};
    use Precedence as P;

    Some(match token {
        Token::Eq => (P::Equals, Binary(InfixOp::Eq)),
        Token::NotEq => (P::Equals, Binary(InfixOp::NotEq)),
        Token::Lt => (P::LessGreater, Binary(InfixOp::Lt)),
        Token::Gt => (P::LessGreater, Binary(InfixOp::Gt)),
        Token::Plus => (P::Sum, Binary(InfixOp::Plus)),
        Token::Minus => (P::Sum, Binary(InfixOp::Minus)),
        Token::Slash => (P::Product, Binary(InfixOp::Slash)),
        Token::Asterisk => (P::Product, Binary(InfixOp::Asterisk)),
        Token::LParen => (P::Call, Call),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;
    use alloc::vec;

    use super::*;

    #[test]
    fn test_string() {
        let program = Program(vec![Stmt::Let { name: "myVar", value: Expr::Ident("anotherVar") }]);
        assert_eq!(program.to_string(), "(let myVar anotherVar)");
    }
}
