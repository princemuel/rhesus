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
        let Some((_start, ch)) = self.chars.next() else {
            return Token::Eof;
        };

        match ch {
            '=' => Token::Assign,
            ';' => Token::Semicolon,
            '(' => Token::LParen,
            ')' => Token::RParen,
            ',' => Token::Comma,
            '+' => Token::Plus,
            '{' => Token::LBrace,
            '}' => Token::RBrace,
            _ => Token::Illegal(ch),
        }
    }

    fn peek_char(&mut self) -> Option<char> { self.chars.peek().map(|&(_, ch)| ch) }
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
    }
}
