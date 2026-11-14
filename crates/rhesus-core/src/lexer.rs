use alloc::borrow::Cow;
use alloc::string::String;
use core::iter::FusedIterator;

use crate::token::Token;

#[derive(Clone, Debug)]
pub struct Lexer<'a> {
    rest: &'a str,
}

impl<'a> Lexer<'a> {
    #[must_use]
    pub const fn new(input: &'a str) -> Self { Self { rest: input } }

    pub fn next_token(&mut self) -> Token<'a> {
        self.skip_whitespace();

        let Some(ch) = self.peek() else {
            return Token::Eof;
        };

        if is_letter(ch) {
            return Token::lookup_ident(self.consume_while(is_letter));
        }
        if ch.is_ascii_digit() {
            return Token::Int(self.consume_while(|c| c.is_ascii_digit()));
        }

        self.advance();
        match ch {
            '=' => self.either('=', Token::Eq, Token::Assign),
            '+' => Token::Plus,
            '-' => Token::Minus,
            '!' => self.either('=', Token::NotEq, Token::Bang),
            '/' => Token::Slash,
            '*' => Token::Asterisk,
            '<' => Token::Lt,
            '>' => Token::Gt,
            ';' => Token::Semicolon,
            ':' => Token::Colon,
            ',' => Token::Comma,
            '{' => Token::LBrace,
            '}' => Token::RBrace,
            '(' => Token::LParen,
            ')' => Token::RParen,
            '[' => Token::LBracket,
            ']' => Token::RBracket,
            '"' => self.read_string(),
            _ => Token::Illegal(ch),
        }
    }

    /// Next char, without consuming it.
    fn peek(&self) -> Option<char> { self.rest.chars().next() }

    /// Consume and return the next char.
    fn bump(&mut self) -> Option<char> {
        let mut chars = self.rest.chars();
        let ch = chars.next()?;
        self.rest = chars.as_str();
        Some(ch)
    }

    /// Consume the next char, if any.
    fn advance(&mut self) { let _ = self.bump(); }

    fn either(&mut self, expected: char, matched: Token<'a>, unmatched: Token<'a>) -> Token<'a> {
        match self.rest.strip_prefix(expected) {
            Some(tail) => {
                self.rest = tail;
                matched
            }
            None => unmatched,
        }
    }

    fn skip_whitespace(&mut self) {
        self.rest = self.rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
    }

    /// Split off the longest prefix whose chars all satisfy `predicate`.
    fn consume_while(&mut self, predicate: impl Fn(char) -> bool) -> &'a str {
        let rest = self.rest;
        let end = rest.find(|ch: char| !predicate(ch)).unwrap_or(rest.len());
        let (token, tail) = rest.split_at(end);
        self.rest = tail;
        token
    }

    /// Called after the opening `"` has been consumed.
    fn read_string(&mut self) -> Token<'a> {
        let mut escaped: Option<String> = None;

        loop {
            let rest = self.rest;
            let Some(idx) = rest.find(['"', '\\']) else {
                self.rest = ""; // unterminated; swallow the remainder
                return Token::Illegal('"');
            };
            let (chunk, tail) = rest.split_at(idx);
            self.rest = tail;

            // `tail` starts with the delimiter that `find` located.
            if self.bump() == Some('"') {
                return Token::String(match escaped {
                    None => Cow::Borrowed(chunk),
                    Some(mut buf) => {
                        buf.push_str(chunk);
                        Cow::Owned(buf)
                    }
                });
            }

            // Otherwise it was a backslash
            let buf = escaped.get_or_insert_with(String::new);
            buf.push_str(chunk);

            let Some(esc) = self.bump() else {
                return Token::Illegal('\\');
            };

            buf.push(match esc {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                other => other, // covers `\"`, `\\`, and unknown escapes
            });
        }
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let token = self.next_token();
        if token == Token::Eof { None } else { Some(token) }
    }
}
impl FusedIterator for Lexer<'_> {}

const fn is_letter(ch: char) -> bool { ch.is_ascii_alphabetic() || ch == '_' }

#[cfg(test)]
mod tests {
    use alloc::borrow::ToOwned as _;
    use alloc::vec;

    use super::*;

    #[test]
    fn scenario_next_token() {
        let input = "=+(){},;";

        let expected = vec![
            Token::Assign,
            Token::Plus,
            Token::LParen,
            Token::RParen,
            Token::LBrace,
            Token::RBrace,
            Token::Comma,
            Token::Semicolon,
            Token::Eof,
        ];

        let mut lexer = Lexer::new(input);
        for token in expected {
            assert_eq!(lexer.next_token(), token);
        }

        let input = "let five = 5;
let ten = 10;

let add = fn(x, y) {
    x + y;
};

let result = add(five, ten);
";

        let expected = vec![
            Token::Let,
            Token::Ident("five"),
            Token::Assign,
            Token::Int("5"),
            Token::Semicolon,
            Token::Let,
            Token::Ident("ten"),
            Token::Assign,
            Token::Int("10"),
            Token::Semicolon,
            Token::Let,
            Token::Ident("add"),
            Token::Assign,
            Token::Function,
            Token::LParen,
            Token::Ident("x"),
            Token::Comma,
            Token::Ident("y"),
            Token::RParen,
            Token::LBrace,
            Token::Ident("x"),
            Token::Plus,
            Token::Ident("y"),
            Token::Semicolon,
            Token::RBrace,
            Token::Semicolon,
            Token::Let,
            Token::Ident("result"),
            Token::Assign,
            Token::Ident("add"),
            Token::LParen,
            Token::Ident("five"),
            Token::Comma,
            Token::Ident("ten"),
            Token::RParen,
            Token::Semicolon,
            Token::Eof,
        ];

        let mut lexer = Lexer::new(input);
        for token in expected {
            assert_eq!(lexer.next_token(), token);
        }

        let input = r#"let five = 5;
let ten = 10;

let add = fn(x, y) {
    x + y;
};

let result = add(five, ten);
!-/*5;
5 < 10 > 5;

if (5 < 10) {
    return true;
} else {
    return false;
}

10 == 10;
10 != 9;
"foobar"
"foo bar"
[1, 2];
{"foo": "bar"}
macro(x, y) { x + y; };
"#;

        let expected = vec![
            Token::Let,
            Token::Ident("five"),
            Token::Assign,
            Token::Int("5"),
            Token::Semicolon,
            Token::Let,
            Token::Ident("ten"),
            Token::Assign,
            Token::Int("10"),
            Token::Semicolon,
            Token::Let,
            Token::Ident("add"),
            Token::Assign,
            Token::Function,
            Token::LParen,
            Token::Ident("x"),
            Token::Comma,
            Token::Ident("y"),
            Token::RParen,
            Token::LBrace,
            Token::Ident("x"),
            Token::Plus,
            Token::Ident("y"),
            Token::Semicolon,
            Token::RBrace,
            Token::Semicolon,
            Token::Let,
            Token::Ident("result"),
            Token::Assign,
            Token::Ident("add"),
            Token::LParen,
            Token::Ident("five"),
            Token::Comma,
            Token::Ident("ten"),
            Token::RParen,
            Token::Semicolon,
            Token::Bang,
            Token::Minus,
            Token::Slash,
            Token::Asterisk,
            Token::Int("5"),
            Token::Semicolon,
            Token::Int("5"),
            Token::Lt,
            Token::Int("10"),
            Token::Gt,
            Token::Int("5"),
            Token::Semicolon,
            Token::If,
            Token::LParen,
            Token::Int("5"),
            Token::Lt,
            Token::Int("10"),
            Token::RParen,
            Token::LBrace,
            Token::Return,
            Token::True,
            Token::Semicolon,
            Token::RBrace,
            Token::Else,
            Token::LBrace,
            Token::Return,
            Token::False,
            Token::Semicolon,
            Token::RBrace,
            Token::Int("10"),
            Token::Eq,
            Token::Int("10"),
            Token::Semicolon,
            Token::Int("10"),
            Token::NotEq,
            Token::Int("9"),
            Token::Semicolon,
            Token::String(Cow::Borrowed("foobar")),
            Token::String(Cow::Borrowed("foo bar")),
            Token::LBracket,
            Token::Int("1"),
            Token::Comma,
            Token::Int("2"),
            Token::RBracket,
            Token::Semicolon,
            Token::LBrace,
            Token::String(Cow::Borrowed("foo")),
            Token::Colon,
            Token::String(Cow::Borrowed("bar")),
            Token::RBrace,
            Token::Macro,
            Token::LParen,
            Token::Ident("x"),
            Token::Comma,
            Token::Ident("y"),
            Token::RParen,
            Token::LBrace,
            Token::Ident("x"),
            Token::Plus,
            Token::Ident("y"),
            Token::Semicolon,
            Token::RBrace,
            Token::Semicolon,
            Token::Eof,
        ];

        let mut lexer = Lexer::new(input);
        for token in expected {
            assert_eq!(lexer.next_token(), token);
        }

        let input = r#"let five = 5;
let add = fn(x, y) { x + y; };
"hello world"
"hello\nworld"
10 == 10; 10 != 9;"#;

        let expected = vec![
            Token::Let,
            Token::Ident("five"),
            Token::Assign,
            Token::Int("5"),
            Token::Semicolon,
            Token::Let,
            Token::Ident("add"),
            Token::Assign,
            Token::Function,
            Token::LParen,
            Token::Ident("x"),
            Token::Comma,
            Token::Ident("y"),
            Token::RParen,
            Token::LBrace,
            Token::Ident("x"),
            Token::Plus,
            Token::Ident("y"),
            Token::Semicolon,
            Token::RBrace,
            Token::Semicolon,
            Token::String(Cow::Borrowed("hello world")),
            Token::String(Cow::Owned("hello\nworld".to_owned())),
            Token::Int("10"),
            Token::Eq,
            Token::Int("10"),
            Token::Semicolon,
            Token::Int("10"),
            Token::NotEq,
            Token::Int("9"),
            Token::Semicolon,
            Token::Eof,
        ];

        let mut lexer = Lexer::new(input);
        for exp in expected {
            assert_eq!(lexer.next_token(), exp);
        }
    }
}
