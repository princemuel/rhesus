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
            return lookup_ident(self.consume_while(is_letter));
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
        let Some(tail) = self.rest.strip_prefix(expected) else { return unmatched };
        self.rest = tail;
        matched
    }

    fn skip_whitespace(&mut self) {
        self.rest = self.rest.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
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

            if self.bump() == Some('"') {
                let Some(mut buf) = escaped else {
                    return Token::String(Cow::Borrowed(chunk));
                };
                buf.push_str(chunk);
                return Token::String(Cow::Owned(buf));
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

#[must_use]
pub const fn lookup_ident(ident: &str) -> Token<'_> {
    match ident {
        "fn" => Token::Function,
        "let" => Token::Let,
        "true" => Token::True,
        "false" => Token::False,
        "if" => Token::If,
        "else" => Token::Else,
        "return" => Token::Return,
        "macro" => Token::Macro,
        _ => Token::Ident(ident),
    }
}

#[cfg(test)]
mod tests {
    use alloc::borrow::ToOwned as _;
    use alloc::vec;

    use super::*;
    use crate::token::Token::*;

    #[test]
    fn scenario_next_token() {
        let input = "=+(){},;";

        let expected = vec![Assign, Plus, LParen, RParen, LBrace, RBrace, Comma, Semicolon, Eof];

        let mut lexer = Lexer::new(input);
        for token in expected {
            assert_eq!(lexer.next_token(), token);
        }

        let input = r#"
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

        #[rustfmt::skip]
        let expected = vec![
            Bang, Minus, Slash, Asterisk, Int("5"), Semicolon,
            Int("5"), Lt, Int("10"), Gt, Int("5"), Semicolon,
            If, LParen, Int("5"), Lt, Int("10"), RParen,
            LBrace, Return, True, Semicolon, RBrace,
            Else, LBrace, Return, False, Semicolon, RBrace,
            Int("10"), Eq, Int("10"), Semicolon,
            Int("10"), NotEq, Int("9"), Semicolon,
            String(Cow::Borrowed("foobar")),
            String(Cow::Borrowed("foo bar")),
            LBracket, Int("1"), Comma, Int("2"), RBracket, Semicolon,
            LBrace, String(Cow::Borrowed("foo")), Colon, String(Cow::Borrowed("bar")), RBrace,
            Macro, LParen, Ident("x"), Comma, Ident("y"),
            RParen, LBrace, Ident("x"), Plus, Ident("y"),
            Semicolon, RBrace, Semicolon,
            Eof,
        ];

        let mut lexer = Lexer::new(input);
        for token in expected {
            assert_eq!(lexer.next_token(), token);
        }

        let input = r#"
"hello world"
"hello\nworld"
10 == 10; 10 != 9;"#;

        let expected = vec![
            String(Cow::Borrowed("hello world")),
            String(Cow::Owned("hello\nworld".to_owned())),
            Int("10"),
            Eq,
            Int("10"),
            Semicolon,
            Int("10"),
            NotEq,
            Int("9"),
            Semicolon,
            Eof,
        ];

        let mut lexer = Lexer::new(input);
        for exp in expected {
            assert_eq!(lexer.next_token(), exp);
        }

        let input = "let five = 5;
let ten = 10;

let add = fn(x, y) {
  x + y;
};

let result = add(five, ten);
!-/*5;
5 < 10 > 5;

if (5 < 10) {
\treturn true;
} else {
\treturn false;
}

10 == 10;
10 != 9;
";
        #[rustfmt::skip]
        let expected = [
            Let, Ident("five"), Assign, Int("5"), Semicolon,
            Let, Ident("ten"), Assign, Int("10"), Semicolon,
            Let, Ident("add"), Assign, Function, LParen, Ident("x"), Comma, Ident("y"), RParen,
            LBrace, Ident("x"), Plus, Ident("y"), Semicolon, RBrace, Semicolon,
            Let, Ident("result"), Assign, Ident("add"), LParen, Ident("five"), Comma,
            Ident("ten"), RParen, Semicolon,
            Bang, Minus, Slash, Asterisk, Int("5"), Semicolon,
            Int("5"), Lt, Int("10"), Gt, Int("5"), Semicolon,
            If, LParen, Int("5"), Lt, Int("10"), RParen, LBrace, Return, True, Semicolon, RBrace,
            Else, LBrace, Return, False, Semicolon, RBrace,
            Int("10"), Eq, Int("10"), Semicolon,
            Int("10"), NotEq, Int("9"), Semicolon,
            Eof,
        ];

        let mut lexer = Lexer::new(input);
        for (i, want) in expected.into_iter().enumerate() {
            assert_eq!(lexer.next_token(), want, "tests[{i}]");
        }
    }
}
