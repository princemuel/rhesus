use alloc::boxed::Box;
use alloc::vec::Vec;
use core::iter::Peekable;

use crate::ast::{Block, Expr, Infix, Precedence, PrefixOp, Stmt, infix_rule};
use crate::errors::{ParseError, ParseResult};
use crate::lexer::Lexer;
use crate::token::Token;

#[derive(Debug)]
pub struct Parser<'a> {
    tokens: Peekable<Lexer<'a>>,
}

impl<'a> Parser<'a> {
    #[must_use]
    pub fn new(lexer: Lexer<'a>) -> Self { Self { tokens: lexer.peekable() } }

    /// Parse everything, collecting every error instead of stopping at the
    /// first.
    ///
    /// # Errors
    ///
    /// Returns a `Vec<ParseError>` containing every syntax error encountered
    /// if any statement fails to parse.
    pub fn parse_program(mut self) -> Result<Block<'a>, Vec<ParseError<'a>>> {
        let mut stmts = Vec::new();
        let mut errors = Vec::new();

        while self.tokens.peek().is_some() {
            match self.parse_stmt() {
                Ok(stmt) => stmts.push(stmt),
                Err(err) => {
                    if !err.ended_at_semicolon() {
                        self.skip_past_semicolon();
                    }
                    errors.push(err);
                }
            }
        }

        if errors.is_empty() { Ok(Block(stmts)) } else { Err(errors) }
    }

    fn parse_block(&mut self) -> ParseResult<'a, Block<'a>> {
        self.expect(&Token::LBrace, "'{'")?;

        let mut stmts = Vec::new();
        while !self.eat(&Token::RBrace) {
            if self.tokens.peek().is_none() {
                return Err(ParseError::UnexpectedEof { expected: "'}'" });
            }

            stmts.push(self.parse_stmt()?);
        }

        Ok(Block(stmts))
    }

    fn parse_stmt(&mut self) -> ParseResult<'a, Stmt<'a>> {
        let stmt = match self.tokens.peek() {
            Some(Token::Let) => self.parse_let()?,
            Some(Token::Return) => self.parse_return()?,
            _ => Stmt::Expr(self.parse_expr(Precedence::Lowest)?),
        };
        let _ = self.eat(&Token::Semicolon); // the `;` is optional
        Ok(stmt)
    }

    fn parse_let(&mut self) -> ParseResult<'a, Stmt<'a>> {
        self.advance(); // `let`
        let name = self.expect_ident()?;
        self.expect(&Token::Assign, "'='")?;
        let value = self.parse_expr(Precedence::Lowest)?;
        Ok(Stmt::Let { name, value })
    }

    fn parse_return(&mut self) -> ParseResult<'a, Stmt<'a>> {
        self.advance(); // `return`
        Ok(Stmt::Return(self.parse_expr(Precedence::Lowest)?))
    }

    fn parse_expr(&mut self, min: Precedence) -> ParseResult<'a, Expr<'a>> {
        let mut left = self.parse_prefix()?;

        while let Some((prec, rule)) = self.tokens.peek().and_then(infix_rule) {
            if prec <= min {
                break;
            }
            self.advance(); // the operator or `(`

            left = match rule {
                Infix::Binary(op) => {
                    Expr::Infix { lhs: Box::new(left), op, rhs: Box::new(self.parse_expr(prec)?) }
                }
                Infix::Call => Expr::Call {
                    function: Box::new(left),
                    args: self.parse_list(&Token::RParen, |p| p.parse_expr(Precedence::Lowest))?,
                },
                Infix::Index => {
                    let index = self.parse_expr(Precedence::Lowest)?;
                    self.expect(&Token::RBracket, "']'")?;
                    Expr::Index { lhs: Box::new(left), index: Box::new(index) }
                }
            };
        }
        Ok(left)
    }

    fn parse_prefix(&mut self) -> ParseResult<'a, Expr<'a>> {
        match self.next("expr")? {
            Token::Ident(name) => Ok(Expr::Ident(name)),
            Token::Int(val) => {
                val.parse().map(Expr::Int).map_err(|_err| ParseError::InvalidInt(val))
            }
            Token::String(s) => Ok(Expr::String(s)),
            Token::True => Ok(Expr::Boolean(true)),
            Token::False => Ok(Expr::Boolean(false)),
            Token::Bang => self.parse_unary(PrefixOp::Bang),
            Token::Minus => self.parse_unary(PrefixOp::Minus),
            Token::LParen => {
                let inner = self.parse_expr(Precedence::Lowest)?;
                self.expect(&Token::RParen, "')'")?;
                Ok(inner)
            }
            Token::If => self.parse_if(),
            Token::Function => self.parse_function(),
            Token::LBracket => Ok(Expr::Array(
                self.parse_list(&Token::RBracket, |p| p.parse_expr(Precedence::Lowest))?,
            )),
            Token::LBrace => self.parse_hash(),
            found => Err(ParseError::UnexpectedToken { expected: "expr", got: found }),
        }
    }

    fn parse_unary(&mut self, op: PrefixOp) -> ParseResult<'a, Expr<'a>> {
        let right = Box::new(self.parse_expr(Precedence::Prefix)?);
        Ok(Expr::Prefix { op, rhs: right })
    }

    fn parse_if(&mut self) -> ParseResult<'a, Expr<'a>> {
        self.expect(&Token::LParen, "'('")?;

        let condition = Box::new(self.parse_expr(Precedence::Lowest)?);
        self.expect(&Token::RParen, "')'")?;

        let consequence = self.parse_block()?;
        let alternative = self.eat(&Token::Else).then(|| self.parse_block()).transpose()?;
        Ok(Expr::If { condition, consequence, alternative })
    }

    fn parse_function(&mut self) -> ParseResult<'a, Expr<'a>> {
        self.expect(&Token::LParen, "'('")?;
        let params = self.parse_list(&Token::RParen, Self::expect_ident)?;
        let body = self.parse_block()?;
        Ok(Expr::Function { params, body })
    }

    /// `item, item, ... close`, with the opening delimiter already consumed.
    fn parse_list<T>(
        &mut self,
        close: &Token<'a>,
        mut item: impl FnMut(&mut Self) -> ParseResult<'a, T>,
    ) -> ParseResult<'a, Vec<T>> {
        let mut items = Vec::new();

        if self.eat(close) {
            return Ok(items);
        }

        loop {
            items.push(item(self)?);

            if self.eat(close) {
                return Ok(items);
            }

            self.expect(&Token::Comma, "',' or closing delimiter")?;
        }
    }

    fn parse_hash(&mut self) -> ParseResult<'a, Expr<'a>> {
        let pairs = self.parse_list(&Token::RBrace, |p| {
            let key = p.parse_expr(Precedence::Lowest)?;
            p.expect(&Token::Colon, "':'")?;

            let value = p.parse_expr(Precedence::Lowest)?;
            Ok((key, value))
        })?;
        Ok(Expr::Hash(pairs))
    }

    fn advance(&mut self) { drop(self.tokens.next()); }

    /// Consume the next token if it equals `expected`.
    fn eat(&mut self, expected: &Token<'a>) -> bool { self.tokens.next_if_eq(expected).is_some() }

    fn next(&mut self, expected: &'static str) -> ParseResult<'a, Token<'a>> {
        self.tokens.next().ok_or(ParseError::UnexpectedEof { expected })
    }

    fn expect(&mut self, expected: &Token<'a>, what: &'static str) -> ParseResult<'a, ()> {
        let found = self.next(what)?;
        let true = found == *expected else {
            return Err(ParseError::UnexpectedToken { expected: what, got: found });
        };

        Ok(())
    }

    fn expect_ident(&mut self) -> ParseResult<'a, &'a str> {
        match self.next("identifier")? {
            Token::Ident(name) => Ok(name),
            found => Err(ParseError::UnexpectedToken { expected: "identifier", got: found }),
        }
    }

    /// Error recovery: drop tokens through the next `;`.
    fn skip_past_semicolon(&mut self) {
        let _ = self.tokens.by_ref().any(|token| token == Token::Semicolon);
    }
}

#[cfg(test)]
mod tests {
    use alloc::borrow::Cow;
    use alloc::string::{String, ToString};
    use alloc::vec;

    use super::*;
    use crate::ast::{InfixOp, Program};

    fn s_expr(src: &str) -> String { Program(stmts(src)).to_string() }

    fn single_expr(src: &str) -> Expr<'_> {
        let mut parsed = stmts(src);
        assert_eq!(parsed.len(), 1, "program should contain 1 statement: {parsed:?}");
        match parsed.remove(0) {
            Stmt::Expr(expr) => expr,
            other => panic!("expected expression statement, got {other:?}"),
        }
    }

    fn stmts(src: &str) -> Vec<Stmt<'_>> {
        let Block(stmts) = Parser::new(Lexer::new(src)).parse_program().expect("parse error");
        stmts
    }

    fn errors(src: &str) -> Vec<ParseError<'_>> {
        Parser::new(Lexer::new(src)).parse_program().expect_err("expected parse errors")
    }

    fn string_lit(s: &str) -> Expr<'_> { Expr::String(Cow::Borrowed(s)) }

    fn int(value: i64) -> Expr<'static> { Expr::Int(value) }

    fn ident(name: &str) -> Expr<'_> { Expr::Ident(name) }

    fn boolean(value: bool) -> Expr<'static> { Expr::Boolean(value) }

    fn prefix(op: PrefixOp, right: Expr<'_>) -> Expr<'_> {
        Expr::Prefix { op, rhs: Box::new(right) }
    }

    fn infix<'a>(left: Expr<'a>, op: InfixOp, right: Expr<'a>) -> Expr<'a> {
        Expr::Infix { lhs: Box::new(left), op, rhs: Box::new(right) }
    }

    #[test]
    fn product_binds_tighter_than_sum() {
        assert_eq!(s_expr("1 + 2 * 3"), "(+ 1 (* 2 3))");
    }

    #[test]
    fn prefix_binds_tighter_than_product() {
        assert_eq!(s_expr("-a * b"), "(* (- a) b)");
    }

    #[test]
    fn reports_all_errors() {
        let errors = Parser::new(Lexer::new("let = 5; let y 6; 7")).parse_program().unwrap_err();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn operator_precedence_formatting() {
        assert_eq!(s_expr("-a * b + c == d / e"), "(== (+ (* (- a) b) c) (/ d e))");
    }

    #[test]
    fn let_statements() {
        let tests = [
            ("let x = 5;", "x", int(5)),
            ("let y = true;", "y", boolean(true)),
            ("let foobar = y;", "foobar", ident("y")),
        ];
        for (input, name, value) in tests {
            assert_eq!(stmts(input), [Stmt::Let { name, value }], "input: {input}");
        }
    }

    #[test]
    fn return_statements() {
        let tests = [
            ("return 5;", int(5)),
            ("return true;", boolean(true)),
            ("return foobar;", ident("foobar")),
        ];
        for (input, value) in tests {
            assert_eq!(stmts(input), [Stmt::Return(value)], "input: {input}");
        }
    }

    #[test]
    fn identifier_expression() {
        assert_eq!(single_expr("foobar;"), ident("foobar"));
    }

    #[test]
    fn integer_literal_expression() {
        assert_eq!(single_expr("5;"), int(5));
    }

    #[test]
    fn boolean_expression() {
        for (input, value) in [("true;", true), ("false;", false)] {
            assert_eq!(single_expr(input), boolean(value), "input: {input}");
        }
    }

    #[test]
    fn parsing_prefix_expressions() {
        let tests = [
            ("!5;", PrefixOp::Bang, int(5)),
            ("-15;", PrefixOp::Minus, int(15)),
            ("!foobar;", PrefixOp::Bang, ident("foobar")),
            ("-foobar;", PrefixOp::Minus, ident("foobar")),
            ("!true;", PrefixOp::Bang, boolean(true)),
            ("!false;", PrefixOp::Bang, boolean(false)),
        ];
        for (input, op, right) in tests {
            assert_eq!(single_expr(input), prefix(op, right), "input: {input}");
        }
    }

    #[test]
    fn parsing_infix_expressions() {
        use InfixOp::{Asterisk, Eq, Gt, Lt, Minus, NotEq, Plus, Slash};

        let tests = [
            ("5 + 5;", int(5), Plus, int(5)),
            ("5 - 5;", int(5), Minus, int(5)),
            ("5 * 5;", int(5), Asterisk, int(5)),
            ("5 / 5;", int(5), Slash, int(5)),
            ("5 > 5;", int(5), Gt, int(5)),
            ("5 < 5;", int(5), Lt, int(5)),
            ("5 == 5;", int(5), Eq, int(5)),
            ("5 != 5;", int(5), NotEq, int(5)),
            ("foobar + barfoo;", ident("foobar"), Plus, ident("barfoo")),
            ("foobar - barfoo;", ident("foobar"), Minus, ident("barfoo")),
            ("foobar * barfoo;", ident("foobar"), Asterisk, ident("barfoo")),
            ("foobar / barfoo;", ident("foobar"), Slash, ident("barfoo")),
            ("foobar > barfoo;", ident("foobar"), Gt, ident("barfoo")),
            ("foobar < barfoo;", ident("foobar"), Lt, ident("barfoo")),
            ("foobar == barfoo;", ident("foobar"), Eq, ident("barfoo")),
            ("foobar != barfoo;", ident("foobar"), NotEq, ident("barfoo")),
            ("true == true", boolean(true), Eq, boolean(true)),
            ("true != false", boolean(true), NotEq, boolean(false)),
            ("false == false", boolean(false), Eq, boolean(false)),
        ];
        for (input, left, op, right) in tests {
            assert_eq!(single_expr(input), infix(left, op, right), "input: {input}");
        }
    }

    #[test]
    fn operator_precedence_parsing() {
        let tests = [
            ("-a * b", "(* (- a) b)"),
            ("!-a", "(! (- a))"),
            ("a + b + c", "(+ (+ a b) c)"),
            ("a + b - c", "(- (+ a b) c)"),
            ("a * b * c", "(* (* a b) c)"),
            ("a * b / c", "(/ (* a b) c)"),
            ("a + b / c", "(+ a (/ b c))"),
            ("a + b * c + d / e - f", "(- (+ (+ a (* b c)) (/ d e)) f)"),
            ("3 + 4; -5 * 5", "(+ 3 4) (* (- 5) 5)"),
            ("5 > 4 == 3 < 4", "(== (> 5 4) (< 3 4))"),
            ("5 < 4 != 3 > 4", "(!= (< 5 4) (> 3 4))"),
            ("3 + 4 * 5 == 3 * 1 + 4 * 5", "(== (+ 3 (* 4 5)) (+ (* 3 1) (* 4 5)))"),
            ("true", "true"),
            ("false", "false"),
            ("3 > 5 == false", "(== (> 3 5) false)"),
            ("3 < 5 == true", "(== (< 3 5) true)"),
            ("1 + (2 + 3) + 4", "(+ (+ 1 (+ 2 3)) 4)"),
            ("(5 + 5) * 2", "(* (+ 5 5) 2)"),
            ("2 / (5 + 5)", "(/ 2 (+ 5 5))"),
            ("(5 + 5) * 2 * (5 + 5)", "(* (* (+ 5 5) 2) (+ 5 5))"),
            ("-(5 + 5)", "(- (+ 5 5))"),
            ("!(true == true)", "(! (== true true))"),
            ("a + add(b * c) + d", "(+ (+ a (call add (* b c))) d)"),
            (
                "add(a, b, 1, 2 * 3, 4 + 5, add(6, 7 * 8))",
                "(call add a b 1 (* 2 3) (+ 4 5) (call add 6 (* 7 8)))",
            ),
            ("add(a + b + c * d / f + g)", "(call add (+ (+ (+ a b) (/ (* c d) f)) g))"),
            ("a * [1, 2, 3, 4][b * c] * d", "(* (* a (index (array 1 2 3 4) (* b c))) d)"),
            (
                "add(a * b[2], b[1], 2 * [1, 2][1])",
                "(call add (* a (index b 2)) (index b 1) (* 2 (index (array 1 2) 1)))",
            ),
        ];
        for (input, want) in tests {
            assert_eq!(s_expr(input), want, "input: {input}");
        }
    }

    #[test]
    fn if_expression() {
        assert_eq!(single_expr("if (x < y) { x }"), Expr::If {
            condition: Box::new(infix(ident("x"), InfixOp::Lt, ident("y"))),
            consequence: Block(vec![Stmt::Expr(ident("x"))]),
            alternative: None,
        });
    }

    #[test]
    fn if_else_expression() {
        assert_eq!(single_expr("if (x < y) { x } else { y }"), Expr::If {
            condition: Box::new(infix(ident("x"), InfixOp::Lt, ident("y"))),
            consequence: Block(vec![Stmt::Expr(ident("x"))]),
            alternative: Some(Block(vec![Stmt::Expr(ident("y"))])),
        });
    }

    #[test]
    fn function_literal_parsing() {
        assert_eq!(single_expr("fn(x, y) { x + y; }"), Expr::Function {
            params: vec!["x", "y"],
            body: Block(vec![Stmt::Expr(infix(ident("x"), InfixOp::Plus, ident("y")))]),
        });
    }

    #[test]
    fn function_parameter_parsing() {
        let tests: [(&str, &[&str]); 3] =
            [("fn() {};", &[]), ("fn(x) {};", &["x"]), ("fn(x, y, z) {};", &["x", "y", "z"])];
        for (input, expected) in tests {
            let Expr::Function { params, .. } = single_expr(input) else {
                panic!("expected function literal for {input}");
            };
            assert_eq!(params, expected, "input: {input}");
        }
    }

    #[test]
    fn call_expression_parsing() {
        assert_eq!(single_expr("add(1, 2 * 3, 4 + 5);"), Expr::Call {
            function: Box::new(ident("add")),
            args: vec![
                int(1),
                infix(int(2), InfixOp::Asterisk, int(3)),
                infix(int(4), InfixOp::Plus, int(5)),
            ],
        });
    }

    #[test]
    fn call_expression_parameter_parsing() {
        let tests: [(&str, &str, &[&str]); 3] = [
            ("add();", "add", &[]),
            ("add(1);", "add", &["1"]),
            ("add(1, 2 * 3, 4 + 5);", "add", &["1", "(* 2 3)", "(+ 4 5)"]),
        ];
        for (input, expected_ident, expected_args) in tests {
            let Expr::Call { function, args } = single_expr(input) else {
                panic!("expected call expression for {input}");
            };
            assert_eq!(*function, ident(expected_ident), "input: {input}");

            let rendered: Vec<_> = args.iter().map(ToString::to_string).collect();
            assert_eq!(rendered, expected_args, "input: {input}");
        }
    }

    #[test]
    fn string_literal_expression() {
        assert_eq!(single_expr(r#""hello world";"#), string_lit("hello world"));
    }

    #[test]
    fn array_literals_parsing() {
        assert_eq!(single_expr("[]"), Expr::Array(vec![]));
        assert_eq!(s_expr("[1, 2 * 2, 3 + 3]"), "(array 1 (* 2 2) (+ 3 3))");
    }

    #[test]
    fn index_expressions_parsing() {
        assert_eq!(s_expr("myArray[1 + 1]"), "(index myArray (+ 1 1))");
    }

    #[test]
    fn hash_literals_parsing() {
        assert_eq!(single_expr("{}"), Expr::Hash(vec![]));
        assert_eq!(
            single_expr(r#"{"one": 1, "two": 2, "three": 3}"#),
            Expr::Hash(vec![
                (string_lit("one"), int(1)),
                (string_lit("two"), int(2)),
                (string_lit("three"), int(3)),
            ])
        );
        assert_eq!(
            single_expr("{true: 1, false: 2}"),
            Expr::Hash(vec![(boolean(true), int(1)), (boolean(false), int(2))])
        );
        assert_eq!(
            single_expr("{1: 1, 2: 2, 3: 3}"),
            Expr::Hash(vec![(int(1), int(1)), (int(2), int(2)), (int(3), int(3))])
        );
        assert_eq!(
            s_expr(r#"{"one": 0 + 1, "two": 10 - 8, "three": 15 / 5}"#),
            r#"(hash ("one" (+ 0 1)) ("two" (- 10 8)) ("three" (/ 15 5)))"#
        );
    }

    #[test]
    fn let_statement_errors() {
        let errs = errors("let x 5;\nlet = 10;\nlet 838383;");
        assert_eq!(errs.len(), 3);
        assert!(matches!(errs[0], ParseError::UnexpectedToken { expected: "'='", .. }));
        assert!(matches!(errs[1], ParseError::UnexpectedToken { expected: "identifier", .. }));
        assert!(matches!(errs[2], ParseError::UnexpectedToken { expected: "identifier", .. }));
    }

    #[test]
    fn integer_literal_out_of_range() {
        assert_eq!(errors("99999999999999999999;"), [ParseError::InvalidInt(
            "99999999999999999999"
        )]);
    }

    #[test]
    fn collects_all_errors() {
        assert_eq!(errors("let = 5; let y 6; 7").len(), 2);
    }

    #[test]
    fn bare_block_is_not_a_statement() {
        // `{` in expression position is a hash literal, so a block fails at the
        // missing `:`.
        let errs = errors("{ x }");
        assert!(matches!(errs[0], ParseError::UnexpectedToken { expected: "':'", .. }));
    }

    #[test]
    fn unterminated_block_is_an_error() {
        let errs = errors("if (x) { x");
        assert_eq!(errs, [ParseError::UnexpectedEof { expected: "'}'" }]);
    }
}
