use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::fmt;

use crate::token::Token;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block<'a>(pub Vec<Stmt<'a>>);

impl<'a> Block<'a> {
    /// Applies [`Expr::try_modify`] to every statement in the block.
    ///
    /// # Errors
    ///
    /// Returns the first error `f` produces.
    pub fn try_modify<E>(
        self,
        f: &mut impl FnMut(Expr<'a>) -> Result<Expr<'a>, E>,
    ) -> Result<Self, E> {
        self.0.into_iter().map(|stmt| stmt.try_modify(f)).collect::<Result<_, E>>().map(Self)
    }

    #[must_use]
    pub fn modify(self, f: &mut impl FnMut(Expr<'a>) -> Expr<'a>) -> Self {
        match self.try_modify(&mut |expr| Ok::<_, !>(f(expr))) {
            Ok(block) => block,
            Err(never) => match never {},
        }
    }
}

impl fmt::Display for Block<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(do")?;
        for stmt in &self.0 {
            write!(f, " {stmt}")?;
        }
        f.write_str(")")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program<'a>(pub Vec<Stmt<'a>>);

impl fmt::Display for Program<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut iter = self.0.iter();
        if let Some(first) = iter.next() {
            write!(f, "{first}")?;
            iter.try_for_each(|item| write!(f, " {item}"))?;
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

impl<'a> Stmt<'a> {
    /// Applies [`Expr::try_modify`] to the expression inside this statement.
    ///
    /// # Errors
    ///
    /// Returns the first error `f` produces.
    pub fn try_modify<E>(
        self,
        f: &mut impl FnMut(Expr<'a>) -> Result<Expr<'a>, E>,
    ) -> Result<Self, E> {
        Ok(match self {
            Self::Let { name, value } => Self::Let { name, value: value.try_modify(f)? },
            Self::Return(value) => Self::Return(value.try_modify(f)?),
            Self::Expr(expr) => Self::Expr(expr.try_modify(f)?),
        })
    }

    #[must_use]
    pub fn modify(self, f: &mut impl FnMut(Expr<'a>) -> Expr<'a>) -> Self {
        match self.try_modify(&mut |expr| Ok::<_, !>(f(expr))) {
            Ok(stmt) => stmt,
            Err(never) => match never {},
        }
    }
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
    Array(Vec<Self>),
    Hash(Vec<(Self, Self)>),
    Index { lhs: Box<Self>, index: Box<Self> },
    Prefix { op: PrefixOp, rhs: Box<Self> },
    Infix { lhs: Box<Self>, op: InfixOp, rhs: Box<Self> },
    If { condition: Box<Self>, consequence: Block<'a>, alternative: Option<Block<'a>> },
    Function { params: Rc<[&'a str]>, body: Rc<Block<'a>> },
    Macro { params: Rc<[&'a str]>, body: Rc<Block<'a>> },
    Call { function: Box<Self>, args: Vec<Self> },
}
impl Expr<'_> {
    /// Rebuilds this expression bottom-up, applying `f` to every sub-expression
    /// (children first, then the node itself, left to right).
    ///
    /// # Errors
    ///
    /// Returns the first error `f` produces; the half-rewritten tree is
    /// dropped.
    pub fn try_modify<E>(
        self,
        modifier: &mut impl FnMut(Self) -> Result<Self, E>,
    ) -> Result<Self, E> {
        let node = match self {
            leaf @ (Self::Ident(_) | Self::Int(_) | Self::String(_) | Self::Boolean(_)) => leaf,
            Self::Array(items) => Self::Array(
                items
                    .into_iter()
                    .map(|item| item.try_modify(modifier))
                    .collect::<Result<_, _>>()?,
            ),
            Self::Hash(pairs) => Self::Hash(
                pairs
                    .into_iter()
                    .map(|(key, value)| {
                        Ok((key.try_modify(modifier)?, value.try_modify(modifier)?))
                    })
                    .collect::<Result<_, _>>()?,
            ),
            Self::Index { lhs, index } => Self::Index {
                lhs: lhs.try_modify(modifier).map(Box::new)?,
                index: index.try_modify(modifier).map(Box::new)?,
            },
            Self::Prefix { op, rhs } => {
                Self::Prefix { op, rhs: rhs.try_modify(modifier).map(Box::new)? }
            }
            Self::Infix { lhs, op, rhs } => Self::Infix {
                lhs: lhs.try_modify(modifier).map(Box::new)?,
                op,
                rhs: rhs.try_modify(modifier).map(Box::new)?,
            },
            Self::If { condition, consequence, alternative } => Self::If {
                condition: condition.try_modify(modifier).map(Box::new)?,
                consequence: consequence.try_modify(modifier)?,
                alternative: alternative.map(|block| block.try_modify(modifier)).transpose()?,
            },
            Self::Function { params, body } => Self::Function {
                params,
                body: Rc::unwrap_or_clone(body).try_modify(modifier).map(Rc::new)?,
            },
            Self::Macro { params, body } => Self::Macro {
                params,
                body: Rc::unwrap_or_clone(body).try_modify(modifier).map(Rc::new)?,
            },
            Self::Call { function, args } => Self::Call {
                function: function.try_modify(modifier).map(Box::new)?,
                args: args
                    .into_iter()
                    .map(|arg| arg.try_modify(modifier))
                    .collect::<Result<_, _>>()?,
            },
        };

        modifier(node)
    }

    /// [`try_modify`](Self::try_modify) for callbacks that can't fail.
    #[must_use]
    pub fn modify(self, f: &mut impl FnMut(Self) -> Self) -> Self {
        match self.try_modify(&mut |expr| Ok::<_, !>(f(expr))) {
            Ok(expr) => expr,
            Err(never) => match never {},
        }
    }
}

#[expect(clippy::use_debug)]
impl fmt::Display for Expr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ident(name) => write!(f, "{name}"),
            Self::Int(val) => write!(f, "{val}"),
            Self::String(s) => write!(f, "{s:?}"),
            Self::Boolean(b) => write!(f, "{b}"),
            Self::Array(items) => {
                f.write_str("(array")?;

                for item in items {
                    write!(f, " {item}")?;
                }

                f.write_str(")")
            }
            Self::Hash(pairs) => {
                f.write_str("(hash")?;

                for (key, value) in pairs {
                    write!(f, " ({key} {value})")?;
                }

                f.write_str(")")
            }
            Self::Index { lhs, index } => write!(f, "(index {lhs} {index})"),
            Self::Prefix { op, rhs: right } => write!(f, "({op} {right})"),
            Self::Infix { lhs: left, op, rhs: right } => write!(f, "({op} {left} {right})"),
            Self::If { condition, consequence, alternative: None } => {
                write!(f, "(if {condition} {consequence})")
            }
            Self::If { condition, consequence, alternative: Some(alt) } => {
                write!(f, "(if {condition} {consequence} {alt})")
            }
            Self::Function { params, body } => {
                write!(f, "(fn ({}) {body})", params.join(" "))
            }
            Self::Macro { params, body } => {
                write!(f, "(macro ({}) {body})", params.join(" "))
            }
            Self::Call { function, args } => {
                write!(f, "(call {function}")?;

                for arg in args {
                    write!(f, " {arg}")?;
                }

                f.write_str(")")
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
    Index,       // a[i]
}

#[derive(Clone, Copy, Debug)]
pub enum Infix {
    Binary(InfixOp),
    Call,
    Index,
}

/// How a token behaves when it appears *after* an expr.
#[must_use]
pub const fn infix_rule(token: &Token<'_>) -> Option<(Precedence, Infix)> {
    use Precedence as P;

    Some(match token {
        Token::Eq => (P::Equals, Infix::Binary(InfixOp::Eq)),
        Token::NotEq => (P::Equals, Infix::Binary(InfixOp::NotEq)),
        Token::Lt => (P::LessGreater, Infix::Binary(InfixOp::Lt)),
        Token::Gt => (P::LessGreater, Infix::Binary(InfixOp::Gt)),
        Token::Plus => (P::Sum, Infix::Binary(InfixOp::Plus)),
        Token::Minus => (P::Sum, Infix::Binary(InfixOp::Minus)),
        Token::Slash => (P::Product, Infix::Binary(InfixOp::Slash)),
        Token::Asterisk => (P::Product, Infix::Binary(InfixOp::Asterisk)),
        Token::LParen => (P::Call, Infix::Call),
        Token::LBracket => (P::Index, Infix::Index),
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

    fn one<'a>() -> Expr<'a> { Expr::Int(1) }
    fn two<'a>() -> Expr<'a> { Expr::Int(2) }

    fn turn_one_into_two(expr: Expr<'_>) -> Expr<'_> { if expr == one() { two() } else { expr } }

    fn boxed(expr: Expr<'_>) -> Box<Expr<'_>> { Box::new(expr) }

    fn block(expr: Expr<'_>) -> Block<'_> { Block(vec![Stmt::Expr(expr)]) }

    fn shared_block(expr: Expr<'_>) -> Rc<Block<'_>> { Rc::new(block(expr)) }

    #[test]
    fn modify_rewrites_every_expression_position() {
        let tests = [
            (one(), two()),
            (
                Expr::Infix { lhs: boxed(one()), op: InfixOp::Plus, rhs: boxed(two()) },
                Expr::Infix { lhs: boxed(two()), op: InfixOp::Plus, rhs: boxed(two()) },
            ),
            (
                Expr::Infix { lhs: boxed(two()), op: InfixOp::Plus, rhs: boxed(one()) },
                Expr::Infix { lhs: boxed(two()), op: InfixOp::Plus, rhs: boxed(two()) },
            ),
            (Expr::Prefix { op: PrefixOp::Minus, rhs: boxed(one()) }, Expr::Prefix {
                op: PrefixOp::Minus,
                rhs: boxed(two()),
            }),
            (Expr::Index { lhs: boxed(one()), index: boxed(one()) }, Expr::Index {
                lhs: boxed(two()),
                index: boxed(two()),
            }),
            (
                Expr::If {
                    condition: boxed(one()),
                    consequence: block(one()),
                    alternative: Some(block(one())),
                },
                Expr::If {
                    condition: boxed(two()),
                    consequence: block(two()),
                    alternative: Some(block(two())),
                },
            ),
            (Expr::Function { params: Rc::from([]), body: shared_block(one()) }, Expr::Function {
                params: Rc::from([]),
                body: shared_block(two()),
            }),
            (Expr::Macro { params: Rc::from([]), body: shared_block(one()) }, Expr::Macro {
                params: Rc::from([]),
                body: shared_block(two()),
            }),
            (Expr::Array(vec![one(), one()]), Expr::Array(vec![two(), two()])),
            // Not in the book: its `Modify` skips call expressions entirely.
            (Expr::Call { function: boxed(one()), args: vec![one(), one()] }, Expr::Call {
                function: boxed(two()),
                args: vec![two(), two()],
            }),
        ];
        for (input, want) in tests {
            let shown = input.to_string();
            assert_eq!(input.modify(&mut turn_one_into_two), want, "input: {shown}");
        }
    }

    #[test]
    fn modify_rewrites_statements() {
        let tests = [
            (Stmt::Expr(one()), Stmt::Expr(two())),
            (Stmt::Return(one()), Stmt::Return(two())),
            (Stmt::Let { name: "x", value: one() }, Stmt::Let { name: "x", value: two() }),
        ];
        for (input, want) in tests {
            assert_eq!(input.modify(&mut turn_one_into_two), want);
        }

        let program = Block(vec![Stmt::Expr(one()), Stmt::Return(one())]);
        let want = Block(vec![Stmt::Expr(two()), Stmt::Return(two())]);
        assert_eq!(program.modify(&mut turn_one_into_two), want);
    }

    #[test]
    fn modify_hash_literals() {
        let hash = Expr::Hash(vec![(one(), one()), (one(), one())]);
        let want = Expr::Hash(vec![(two(), two()), (two(), two())]);
        assert_eq!(hash.modify(&mut turn_one_into_two), want);
    }

    #[test]
    fn modify_visits_children_before_parents_left_to_right() {
        let expr = Expr::Infix {
            lhs: boxed(one()),
            op: InfixOp::Plus,
            rhs: boxed(Expr::Infix { lhs: boxed(two()), op: InfixOp::Asterisk, rhs: boxed(one()) }),
        };
        let mut seen = vec![];
        drop(expr.modify(&mut |e| {
            seen.push(e.to_string());
            e
        }));
        assert_eq!(seen, ["1", "2", "1", "(* 2 1)", "(+ 1 (* 2 1))"]);
    }

    #[test]
    fn try_modify_stops_at_the_first_error() {
        let mut calls = 0;
        let result = Expr::Array(vec![one(), two(), Expr::Int(3)]).try_modify(&mut |e| {
            calls += 1;
            if e == two() { Err("boom") } else { Ok(e) }
        });
        assert_eq!(result, Err("boom"));
        assert_eq!(calls, 2, "the third element must not be visited");
    }

    #[test]
    fn block_and_macro_display() {
        assert_eq!(Block(vec![]).to_string(), "(do)");
        assert_eq!(block(one()).to_string(), "(do 1)");
        let mac =
            Expr::Macro { params: Rc::from(["x", "y"]), body: shared_block(Expr::Ident("x")) };
        assert_eq!(mac.to_string(), "(macro (x y) (do x))");
    }

    #[test]
    fn strings_print_quoted_so_they_differ_from_identifiers() {
        assert_eq!(Expr::String("a b".into()).to_string(), r#""a b""#);
        assert_eq!(Expr::Ident("ab").to_string(), "ab");
    }
}
