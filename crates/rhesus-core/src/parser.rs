use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::iter::Peekable;

use crate::ast::{Block, Expr, Infix, Precedence, PrefixOp, Stmt, infix_rule};
use crate::errors::ParseError;
use crate::lexer::Lexer;
use crate::token::Token;

/// How deep the syntax tree may get.
pub const MAX_SYNTAX_TREE_NESTING: usize = 256;

#[must_use]
pub fn is_incomplete(source: &str) -> bool {
    Parser::new(Lexer::new(source))
        .parse_program()
        .is_err_and(|errors| errors.iter().all(ParseError::is_incomplete))
}

#[derive(Debug)]
pub struct Parser<'a> {
    tokens: Peekable<Lexer<'a>>,
    depth: usize,
}

impl<'a> Parser<'a> {
    #[must_use]
    pub fn new(lexer: Lexer<'a>) -> Self { Self { tokens: lexer.peekable(), depth: 0 } }

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
            let start = self.tokens.clone();

            match self.parse_stmt() {
                Ok(stmt) => stmts.push(stmt),
                Err(error) => {
                    errors.push(error);

                    self.tokens = start;
                    self.skip_statement();
                }
            }
        }

        if errors.is_empty() { Ok(Block(stmts)) } else { Err(errors) }
    }

    fn parse_block(&mut self) -> Result<Block<'a>, ParseError<'a>> {
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

    fn parse_stmt(&mut self) -> Result<Stmt<'a>, ParseError<'a>> {
        let stmt = match self.tokens.peek() {
            Some(Token::Let) => self.parse_let()?,
            Some(Token::Return) => self.parse_return()?,
            _ => Stmt::Expr(self.parse_expr(Precedence::Lowest)?),
        };
        let _ = self.eat(&Token::Semicolon); // the `;` is optional
        Ok(stmt)
    }

    fn parse_let(&mut self) -> Result<Stmt<'a>, ParseError<'a>> {
        self.advance(); // `let`
        let name = self.expect_ident()?;
        self.expect(&Token::Assign, "'='")?;
        let value = self.parse_expr(Precedence::Lowest)?;
        Ok(Stmt::Let { name, value })
    }

    fn parse_return(&mut self) -> Result<Stmt<'a>, ParseError<'a>> {
        self.advance(); // `return`
        Ok(Stmt::Return(self.parse_expr(Precedence::Lowest)?))
    }

    fn parse_expr(&mut self, min: Precedence) -> Result<Expr<'a>, ParseError<'a>> {
        let depth = self.depth;
        let result = self.parse_expr_at_depth(min);
        self.depth = depth;
        result
    }

    const fn deepen(&mut self) -> Result<(), ParseError<'a>> {
        if self.depth >= MAX_SYNTAX_TREE_NESTING {
            return Err(ParseError::NestingTooDeep);
        }
        self.depth += 1;
        Ok(())
    }

    fn parse_expr_at_depth(&mut self, min: Precedence) -> Result<Expr<'a>, ParseError<'a>> {
        self.deepen()?;
        let mut left = self.parse_prefix()?;

        while let Some((prec, rule)) = self.tokens.peek().and_then(infix_rule) {
            if prec <= min {
                break;
            }
            self.advance(); // the operator or `(`
            self.deepen()?; // `left` is about to become a child of a new node

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

    fn parse_prefix(&mut self) -> Result<Expr<'a>, ParseError<'a>> {
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
            Token::Function => {
                let (params, body) = self.parse_params_and_body()?;
                Ok(Expr::Function { params, body })
            }
            Token::Macro => {
                let (params, body) = self.parse_params_and_body()?;
                Ok(Expr::Macro { params, body })
            }
            Token::LBracket => Ok(Expr::Array(
                self.parse_list(&Token::RBracket, |p| p.parse_expr(Precedence::Lowest))?,
            )),
            Token::LBrace => self.parse_hash(),
            found => Err(ParseError::UnexpectedToken { expected: "expr", got: found }),
        }
    }

    fn parse_unary(&mut self, op: PrefixOp) -> Result<Expr<'a>, ParseError<'a>> {
        let right = Box::new(self.parse_expr(Precedence::Prefix)?);
        Ok(Expr::Prefix { op, rhs: right })
    }

    fn parse_if(&mut self) -> Result<Expr<'a>, ParseError<'a>> {
        self.expect(&Token::LParen, "'('")?;

        let condition = Box::new(self.parse_expr(Precedence::Lowest)?);
        self.expect(&Token::RParen, "')'")?;

        let consequence = self.parse_block()?;
        let alternative = if self.eat(&Token::Else) { Some(self.parse_block()?) } else { None };
        Ok(Expr::If { condition, consequence, alternative })
    }

    fn parse_params_and_body(&mut self) -> Result<(Rc<[&'a str]>, Rc<Block<'a>>), ParseError<'a>> {
        self.expect(&Token::LParen, "'('")?;
        let params = self.parse_list(&Token::RParen, Self::expect_ident)?;
        let body = self.parse_block()?;
        Ok((params.into(), Rc::new(body)))
    }

    /// `item, item, ... close`, with the opening delimiter already consumed.
    fn parse_list<T>(
        &mut self,
        close: &Token<'a>,
        mut item: impl FnMut(&mut Self) -> Result<T, ParseError<'a>>,
    ) -> Result<Vec<T>, ParseError<'a>> {
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

    fn parse_hash(&mut self) -> Result<Expr<'a>, ParseError<'a>> {
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

    fn next(&mut self, expected: &'static str) -> Result<Token<'a>, ParseError<'a>> {
        self.tokens.next().ok_or(ParseError::UnexpectedEof { expected })
    }

    fn expect(&mut self, expected: &Token<'a>, what: &'static str) -> Result<(), ParseError<'a>> {
        match self.next(what)? {
            found if found == *expected => Ok(()),
            found => Err(ParseError::UnexpectedToken { expected: what, got: found }),
        }
    }

    fn expect_ident(&mut self) -> Result<&'a str, ParseError<'a>> {
        match self.next("identifier")? {
            Token::Ident(name) => Ok(name),
            found => Err(ParseError::UnexpectedToken { expected: "identifier", got: found }),
        }
    }

    /// Error recovery: drop everything through the bad statement's own `;`
    fn skip_statement(&mut self) {
        let mut open = 0usize;
        for token in self.tokens.by_ref() {
            match token {
                Token::LParen | Token::LBracket | Token::LBrace => open += 1,
                Token::RParen | Token::RBracket | Token::RBrace => open = open.saturating_sub(1),
                Token::Semicolon if open == 0 => return,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::borrow::Cow;
    use alloc::string::{String, ToString};
    use alloc::vec;

    use super::*;
    use crate::ast::{InfixOp, Program};

    fn sexpr(src: &str) -> String { Program(stmts(src)).to_string() }

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

    fn int(value: i64) -> Expr<'static> { Expr::Int(value) }

    fn boolean(value: bool) -> Expr<'static> { Expr::Boolean(value) }

    fn string(s: &str) -> Expr<'_> { Expr::String(Cow::Borrowed(s)) }

    fn ident(name: &str) -> Expr<'_> { Expr::Ident(name) }

    fn prefix(op: PrefixOp, right: Expr<'_>) -> Expr<'_> {
        Expr::Prefix { op, rhs: Box::new(right) }
    }

    fn infix<'a>(left: Expr<'a>, op: InfixOp, right: Expr<'a>) -> Expr<'a> {
        Expr::Infix { lhs: Box::new(left), op, rhs: Box::new(right) }
    }

    #[test]
    fn product_binds_tighter_than_sum() {
        assert_eq!(sexpr("1 + 2 * 3"), "(+ 1 (* 2 3))");
    }

    #[test]
    fn prefix_binds_tighter_than_product() {
        assert_eq!(sexpr("-a * b"), "(* (- a) b)");
    }

    #[test]
    fn reports_all_errors() {
        let errors = Parser::new(Lexer::new("let = 5; let y 6; 7")).parse_program().unwrap_err();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn operator_precedence_formatting() {
        assert_eq!(sexpr("-a * b + c == d / e"), "(== (+ (* (- a) b) c) (/ d e))");
    }

    #[test]
    fn parsing_let_statements() {
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
    fn parsing_return_statements() {
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
    fn parsing_operator_precedence() {
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
            assert_eq!(sexpr(input), want, "input: {input}");
        }
    }

    #[test]
    fn parsing_if_expressions() {
        assert_eq!(single_expr("if (x < y) { x }"), Expr::If {
            condition: Box::new(infix(ident("x"), InfixOp::Lt, ident("y"))),
            consequence: Block(vec![Stmt::Expr(ident("x"))]),
            alternative: None,
        });
    }

    #[test]
    fn parsing_if_else_expressions() {
        assert_eq!(single_expr("if (x < y) { x } else { y }"), Expr::If {
            condition: Box::new(infix(ident("x"), InfixOp::Lt, ident("y"))),
            consequence: Block(vec![Stmt::Expr(ident("x"))]),
            alternative: Some(Block(vec![Stmt::Expr(ident("y"))])),
        });
    }

    #[test]
    fn parsing_function_literals() {
        assert_eq!(single_expr("fn(x, y) { x + y; }"), Expr::Function {
            params: Rc::from(["x", "y"]),
            body: Rc::new(Block(vec![Stmt::Expr(infix(ident("x"), InfixOp::Plus, ident("y")))])),
        });
    }

    #[test]
    fn parsing_function_parameter() {
        let tests: [(&str, &[&str]); 3] =
            [("fn() {};", &[]), ("fn(x) {};", &["x"]), ("fn(x, y, z) {};", &["x", "y", "z"])];
        for (input, expected) in tests {
            let Expr::Function { params, .. } = single_expr(input) else {
                panic!("expected function literal for {input}");
            };
            assert_eq!(&*params, expected, "input: {input}");
        }
    }

    #[test]
    fn parsing_call_expressions() {
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
    fn parsing_call_expression_parameters() {
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
    fn parsing_string_literals() {
        assert_eq!(single_expr(r#""hello world";"#), string("hello world"));
    }

    #[test]
    fn parsing_array_literals() {
        assert_eq!(single_expr("[]"), Expr::Array(vec![]));
        assert_eq!(sexpr("[1, 2 * 2, 3 + 3]"), "(array 1 (* 2 2) (+ 3 3))");
    }

    #[test]
    fn parsing_index_expressions() {
        assert_eq!(sexpr("myArray[1 + 1]"), "(index myArray (+ 1 1))");
    }

    #[test]
    fn parsing_hash_literals() {
        assert_eq!(single_expr("{}"), Expr::Hash(vec![]));
        assert_eq!(
            single_expr(r#"{"one": 1, "two": 2, "three": 3}"#),
            Expr::Hash(vec![
                (string("one"), int(1)),
                (string("two"), int(2)),
                (string("three"), int(3)),
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
            sexpr(r#"{"one": 0 + 1, "two": 10 - 8, "three": 15 / 5}"#),
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
    fn integer_literal_out_of_range_errors() {
        assert_eq!(errors("99999999999999999999;"), [ParseError::InvalidInt(
            "99999999999999999999"
        )]);
    }

    #[test]
    fn collects_all_errors() {
        assert_eq!(errors("let = 5; let y 6; 7").len(), 2);
    }

    #[test]
    fn bare_block_is_not_a_statement_errors() {
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
    #[test]
    fn parsing_macro_literals() {
        assert_eq!(single_expr("macro(x, y) { x + y; }"), Expr::Macro {
            params: Rc::from(["x", "y"]),
            body: Rc::new(Block(vec![Stmt::Expr(infix(ident("x"), InfixOp::Plus, ident("y")))])),
        });
    }

    #[test]
    fn parsing_macro_parameters() {
        let tests: [(&str, &[&str]); 3] = [
            ("macro() {};", &[]),
            ("macro(x) {};", &["x"]),
            ("macro(x, y, z) {};", &["x", "y", "z"]),
        ];
        for (input, expected) in tests {
            let Expr::Macro { params, .. } = single_expr(input) else {
                panic!("expected macro literal for {input}");
            };
            assert_eq!(&*params, expected, "input: {input}");
        }
    }

    #[test]
    fn parsing_macro_definition_statements() {
        assert_eq!(sexpr("let m = macro(x) { x };"), "(let m (macro (x) (do x)))");
    }

    #[test]
    fn macro_errors() {
        assert!(matches!(errors("macro x { }")[0], ParseError::UnexpectedToken {
            expected: "'('",
            ..
        }));
        assert!(matches!(errors("macro(1) { }")[0], ParseError::UnexpectedToken {
            expected: "identifier",
            ..
        }));
    }

    #[test]
    fn blocks_print_balanced() {
        assert_eq!(sexpr("if (x) { 1; 2 } else { }"), "(if x (do 1 2) (do))");
        assert_eq!(sexpr("fn(a) { return a; }"), "(fn (a) (do (return a)))");
    }

    #[test]
    fn an_error_inside_a_block_is_reported_once() {
        // Recovery must skip the whole bad statement, not stop at the `;`
        // inside its block (which would leave `2 }` to produce a second, bogus
        // error).
        let errs = errors("fn() { let = 1; 2 }; 3");
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(matches!(errs[0], ParseError::UnexpectedToken { expected: "identifier", .. }));
    }

    #[test]
    fn parsing_resumes_after_a_bad_block_statement() {
        let errs = errors("fn() { let = 1; 2 }; let y 6; if (a) { let = 3 }; 7");
        assert_eq!(errs.len(), 3, "{errs:?}");
    }

    #[test]
    fn stray_closing_brace_is_one_error() {
        let errs = errors("} let a = 1; let b = 2;");
        assert_eq!(errs.len(), 1, "{errs:?}");
    }

    #[test]
    fn nesting_is_capped_instead_of_overflowing_the_stack() {
        let nested = |n: usize| alloc::format!("{}1{}", "(".repeat(n), ")".repeat(n));

        assert_eq!(single_expr(&nested(MAX_SYNTAX_TREE_NESTING - 1)), int(1));
        assert_eq!(errors(&nested(MAX_SYNTAX_TREE_NESTING)), [ParseError::NestingTooDeep]);

        assert_eq!(errors(&"!".repeat(MAX_SYNTAX_TREE_NESTING + 1)), [ParseError::NestingTooDeep]);
        assert_eq!(errors(&"[".repeat(MAX_SYNTAX_TREE_NESTING + 1)), [ParseError::NestingTooDeep]);
        assert_eq!(errors(&"fn(){".repeat(MAX_SYNTAX_TREE_NESTING + 1)), [
            ParseError::NestingTooDeep
        ]);
    }

    #[test]
    fn long_operator_chains_are_capped() {
        let chain = |operators: usize| alloc::format!("1{}", " + 1".repeat(operators));

        assert!(matches!(single_expr(&chain(MAX_SYNTAX_TREE_NESTING - 2)), Expr::Infix { .. }));
        assert_eq!(errors(&chain(MAX_SYNTAX_TREE_NESTING - 1)), [ParseError::NestingTooDeep]);

        let calls = |n: usize| alloc::format!("f{}", "()".repeat(n));
        assert!(matches!(single_expr(&calls(MAX_SYNTAX_TREE_NESTING - 1)), Expr::Call { .. }));
        assert_eq!(errors(&calls(MAX_SYNTAX_TREE_NESTING)), [ParseError::NestingTooDeep]);
    }

    #[test]
    fn parser_works_again_after_hitting_the_nesting_cap() {
        let src = alloc::format!(
            "{}1{}; 2 + 3",
            "(".repeat(MAX_SYNTAX_TREE_NESTING),
            ")".repeat(MAX_SYNTAX_TREE_NESTING)
        );
        assert_eq!(errors(&src), [ParseError::NestingTooDeep]);
        assert_eq!(sexpr("let a = ((1)); a"), "(let a 1) a");
    }

    #[test]
    fn incomplete_input_is_detected() {
        for src in [
            "if (x) {",
            "if (x) { 1 } else {",
            "let a = [1, 2",
            "foo(",
            "fn(a, b) { a +",
            "let m = {\"a\": 1,",
            "let s = \"unterminated",
            "let s = \"dangling escape\\",
            "1 +",
            "let",
        ] {
            assert!(is_incomplete(src), "should be incomplete: {src:?}");
        }
    }

    #[test]
    fn complete_or_hopeless_input_is_not_incomplete() {
        for src in [
            "",
            "1 + 2",
            "if (x) { 1 }",
            "let = 5",
            "let x 5;",
            "foo(1 2",
            "if (x { 1 }",
            "}",
            "let a = 1; let = 2; if (x) {",
            ".exit",
        ] {
            assert!(!is_incomplete(src), "should not be incomplete: {src:?}");
        }
    }
}
