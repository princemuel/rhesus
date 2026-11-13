use alloc::borrow::Cow;
use alloc::string::String;
use core::iter::Peekable;
use core::str::CharIndices;

use crate::token::Token;

#[derive(Clone, Debug)]
pub struct Lexer<'a> {
    input: &'a str,
    chars: Peekable<CharIndices<'a>>,
}

impl<'a> Lexer<'a> {
    #[must_use]
    pub fn new(input: &'a str) -> Self { Self { input, chars: input.char_indices().peekable() } }

    pub fn next_token(&mut self) -> Token<'a> {
        self.skip_whitespace();

        let Some((start, ch)) = self.chars.next() else {
            return Token::Eof;
        };

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
            '"' => self.read_string(start + 1),
            '[' => Token::LBracket,
            ']' => Token::RBracket,
            c if is_letter(c) => Token::lookup_ident(self.read_while(start, is_letter)),
            c if c.is_ascii_digit() => Token::Int(self.read_while(start, |c| c.is_ascii_digit())),
            _ => Token::Illegal(ch),
        }
    }

    fn either(&mut self, expected: char, matched: Token<'a>, unmatched: Token<'a>) -> Token<'a> {
        if self.peek_char() == Some(expected) {
            let _next = self.chars.next();
            matched
        } else {
            unmatched
        }
    }

    fn peek_char(&mut self) -> Option<char> { self.chars.peek().map(|&(_, ch)| ch) }

    #[expect(clippy::string_slice)]
    fn read_while<P>(&mut self, start: usize, predicate: P) -> &'a str
    where
        P: Fn(char) -> bool,
    {
        let mut end = self.input.len();
        while let Some(&(idx, ch)) = self.chars.peek() {
            if predicate(ch) {
                let _next = self.chars.next();
            } else {
                end = idx;
                break;
            }
        }

        &self.input[start..end]
    }

    #[expect(clippy::string_slice)]
    fn read_string(&mut self, start: usize) -> Token<'a> {
        let mut escaped: Option<String> = None;
        let mut slice_start = start;

        while let Some((idx, ch)) = self.chars.next() {
            match ch {
                '"' => {
                    let Some(mut buf) = escaped else {
                        return Token::String(Cow::Borrowed(&self.input[start..idx]));
                    };
                    buf.push_str(&self.input[slice_start..idx]);

                    return Token::String(Cow::Owned(buf));
                }
                '\\' => {
                    let buf = escaped.get_or_insert_with(String::new);
                    buf.push_str(&self.input[slice_start..idx]);

                    let Some((esc_idx, esc_ch)) = self.chars.next() else {
                        return Token::Illegal('\\');
                    };

                    match esc_ch {
                        'n' => buf.push('\n'),
                        't' => buf.push('\t'),
                        'r' => buf.push('\r'),
                        '"' => buf.push('"'),
                        '\\' => buf.push('\\'),
                        other => buf.push(other),
                    }
                    slice_start = esc_idx + esc_ch.len_utf8();
                }
                _ => {}
            }
        }
        Token::Illegal('"')
    }

    fn skip_whitespace(&mut self) {
        while let Some(&(_, ch)) = self.chars.peek() {
            if ch.is_ascii_whitespace() {
                let _next = self.chars.next();
            } else {
                break;
            }
        }
    }
}

const fn is_letter(ch: char) -> bool { ch.is_ascii_alphabetic() || ch == '_' }

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn test_next_token() {
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
    }
}
