// Lexer — токенизатор Vibra Script.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Литералы
    Number(i64),
    Str(String),
    Ident(String),

    // Ключевые слова
    Var,
    If,
    Else,
    While,
    For,
    To,
    Fn,
    Return,
    Exit,
    Print,
    Beep,
    Sleep,
    Exec,
    Input,
    Let,

    // Операторы
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Assign,
    Not,

    // Разделители
    LParen,
    RParen,
    LBrace,
    RBrace,
    Semicolon,
    Comma,
    Colon,

    // Специальные
    Newline,
    Eof,
}

pub fn tokenize(source: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = source.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];

        // Пробелы и табы — пропускаем
        if ch == ' ' || ch == '\t' || ch == '\r' {
            i += 1;
            continue;
        }

        // Комментарий
        if ch == '#' {
            while i < len && chars[i] != '\n' { i += 1; }
            continue;
        }

        // Перевод строки
        if ch == '\n' {
            tokens.push(Token::Newline);
            i += 1;
            continue;
        }

        // Число
        if ch.is_ascii_digit() {
            let mut num = 0i64;
            while i < len && chars[i].is_ascii_digit() {
                num = num * 10 + (chars[i] as i64 - '0' as i64);
                i += 1;
            }
            tokens.push(Token::Number(num));
            continue;
        }

        // Идентификатор или ключевое слово
        if ch.is_ascii_alphabetic() || ch == '_' {
            let mut ident = String::new();
            while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                ident.push(chars[i]);
                i += 1;
            }
            match ident.as_str() {
                "var" | "let" => tokens.push(Token::Var),
                "if" => tokens.push(Token::If),
                "else" => tokens.push(Token::Else),
                "while" => tokens.push(Token::While),
                "for" => tokens.push(Token::For),
                "to" => tokens.push(Token::To),
                "fn" => tokens.push(Token::Fn),
                "return" | "ret" => tokens.push(Token::Return),
                "exit" | "quit" => tokens.push(Token::Exit),
                "print" | "echo" => tokens.push(Token::Print),
                "input" => tokens.push(Token::Input),
                "beep" | "sound" => tokens.push(Token::Beep),
                "sleep" | "wait" => tokens.push(Token::Sleep),
                "exec" | "run" => tokens.push(Token::Exec),
                "let" => tokens.push(Token::Let),
                "true" => tokens.push(Token::Number(1)),
                "false" => tokens.push(Token::Number(0)),
                "null" | "nil" => tokens.push(Token::Number(0)),
                _ => tokens.push(Token::Ident(ident)),
            }
            continue;
        }

        // Строка
        if ch == '"' || ch == '\'' {
            let quote = ch;
            let mut s = String::new();
            i += 1;
            while i < len && chars[i] != quote {
                if chars[i] == '\\' && i + 1 < len {
                    i += 1;
                    match chars[i] {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        '\'' => s.push('\''),
                        _ => { s.push('\\'); s.push(chars[i]); }
                    }
                } else {
                    s.push(chars[i]);
                }
                i += 1;
            }
            if i < len { i += 1; } // пропускаем закрывающую кавычку
            tokens.push(Token::Str(s));
            continue;
        }

        // Операторы
        match ch {
            '+' => { tokens.push(Token::Plus); i += 1; }
            '-' => { tokens.push(Token::Minus); i += 1; }
            '*' => { tokens.push(Token::Star); i += 1; }
            '/' => { tokens.push(Token::Slash); i += 1; }
            '%' => { tokens.push(Token::Percent); i += 1; }
            '(' => { tokens.push(Token::LParen); i += 1; }
            ')' => { tokens.push(Token::RParen); i += 1; }
            '{' => { tokens.push(Token::LBrace); i += 1; }
            '}' => { tokens.push(Token::RBrace); i += 1; }
            ';' => { tokens.push(Token::Semicolon); i += 1; }
            ',' => { tokens.push(Token::Comma); i += 1; }
            ':' => { tokens.push(Token::Colon); i += 1; }
            '!' => {
                if i + 1 < len && chars[i + 1] == '=' {
                    tokens.push(Token::Ne); i += 2;
                } else {
                    tokens.push(Token::Not); i += 1;
                }
            }
            '=' => {
                if i + 1 < len && chars[i + 1] == '=' {
                    tokens.push(Token::Eq); i += 2;
                } else {
                    tokens.push(Token::Assign); i += 1;
                }
            }
            '<' => {
                if i + 1 < len && chars[i + 1] == '=' {
                    tokens.push(Token::Le); i += 2;
                } else {
                    tokens.push(Token::Lt); i += 1;
                }
            }
            '>' => {
                if i + 1 < len && chars[i + 1] == '=' {
                    tokens.push(Token::Ge); i += 2;
                } else {
                    tokens.push(Token::Gt); i += 1;
                }
            }
            _ => { i += 1; } // пропускаем неизвестные символы
        }
    }

    tokens.push(Token::Eof);
    tokens
}
