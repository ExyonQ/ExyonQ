/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
//! NGINX config lexer with source locations.

use crate::limits::MAX_TOKEN_LENGTH;
use crate::report::SourceLoc;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Word(String),
    LBrace,
    RBrace,
    Semicolon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub loc: SourceLoc,
}

#[derive(Debug, Error)]
pub enum LexError {
    #[error("{file}:{line}:{column}: unterminated string")]
    UnterminatedString {
        file: String,
        line: u32,
        column: u32,
    },
    #[error("{file}:{line}:{column}: token exceeds {max} bytes")]
    TokenTooLong {
        file: String,
        line: u32,
        column: u32,
        max: usize,
    },
}

pub fn lex(file: &str, input: &str) -> Result<Vec<Token>, LexError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;

    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '{' => {
                tokens.push(Token {
                    kind: TokenKind::LBrace,
                    loc: SourceLoc {
                        file: file.into(),
                        line,
                        column: col,
                    },
                });
                i += 1;
                col += 1;
            }
            '}' => {
                tokens.push(Token {
                    kind: TokenKind::RBrace,
                    loc: SourceLoc {
                        file: file.into(),
                        line,
                        column: col,
                    },
                });
                i += 1;
                col += 1;
            }
            ';' => {
                tokens.push(Token {
                    kind: TokenKind::Semicolon,
                    loc: SourceLoc {
                        file: file.into(),
                        line,
                        column: col,
                    },
                });
                i += 1;
                col += 1;
            }
            '"' | '\'' => {
                let start_line = line;
                let start_col = col;
                let quote = ch;
                i += 1;
                col += 1;
                let mut value = String::new();
                while i < chars.len() && chars[i] != quote {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        col += 1;
                        value.push(chars[i]);
                    } else {
                        if chars[i] == '\n' {
                            line += 1;
                            col = 0;
                        }
                        value.push(chars[i]);
                    }
                    i += 1;
                    col += 1;
                }
                if i >= chars.len() || chars[i] != quote {
                    return Err(LexError::UnterminatedString {
                        file: file.into(),
                        line: start_line,
                        column: start_col,
                    });
                }
                i += 1;
                col += 1;
                if value.len() > MAX_TOKEN_LENGTH {
                    return Err(LexError::TokenTooLong {
                        file: file.into(),
                        line: start_line,
                        column: start_col,
                        max: MAX_TOKEN_LENGTH,
                    });
                }
                tokens.push(Token {
                    kind: TokenKind::Word(value),
                    loc: SourceLoc {
                        file: file.into(),
                        line: start_line,
                        column: start_col,
                    },
                });
            }
            '#' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\r' => {
                i += 1;
            }
            '\n' => {
                line += 1;
                col = 0;
                i += 1;
            }
            c if c.is_whitespace() => {
                i += 1;
                col += 1;
            }
            _ => {
                let start_line = line;
                let start_col = col;
                let mut value = String::new();
                value.push(ch);
                i += 1;
                col += 1;
                while i < chars.len() {
                    let next = chars[i];
                    if next.is_whitespace()
                        || next == '{'
                        || next == '}'
                        || next == ';'
                        || next == '#'
                    {
                        break;
                    }
                    value.push(next);
                    i += 1;
                    col += 1;
                }
                if value.len() > MAX_TOKEN_LENGTH {
                    return Err(LexError::TokenTooLong {
                        file: file.into(),
                        line: start_line,
                        column: start_col,
                        max: MAX_TOKEN_LENGTH,
                    });
                }
                tokens.push(Token {
                    kind: TokenKind::Word(value),
                    loc: SourceLoc {
                        file: file.into(),
                        line: start_line,
                        column: start_col,
                    },
                });
            }
        }
    }
    Ok(tokens)
}
