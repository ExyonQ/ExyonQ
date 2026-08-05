//! Parse token stream into NGINX AST.

use crate::ast::{Block, Config, Directive};
use crate::lexer::{Token, TokenKind};
use crate::limits::{MAX_BLOCK_DEPTH, MAX_DIRECTIVES, MAX_TOKENS};
use crate::report::{CompatStatus, CompatibilityReport, SourceLoc};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("{loc:?}: {message}")]
    Syntax { loc: SourceLoc, message: String },
    #[error("{loc:?}: {message}")]
    Limit { loc: SourceLoc, message: String },
}

pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    directive_count: usize,
    _report: &'a mut CompatibilityReport,
}

impl<'a> Parser<'a> {
    pub fn parse(
        tokens: &'a [Token],
        report: &mut CompatibilityReport,
    ) -> Result<Config, ParseError> {
        if tokens.len() > MAX_TOKENS {
            let loc = tokens
                .first()
                .map(|t| t.loc.clone())
                .unwrap_or_else(|| SourceLoc::synthetic("input"));
            return Err(ParseError::Limit {
                loc,
                message: format!("token count exceeds {MAX_TOKENS}"),
            });
        }
        let mut p = Parser {
            tokens,
            pos: 0,
            directive_count: 0,
            _report: report,
        };
        p.parse_config()
    }

    fn parse_config(&mut self) -> Result<Config, ParseError> {
        let mut directives = Vec::new();
        while !self.eof() {
            directives.push(self.parse_directive(0)?);
        }
        Ok(Config { directives })
    }

    fn parse_directive(&mut self, depth: usize) -> Result<Directive, ParseError> {
        if depth > MAX_BLOCK_DEPTH {
            let loc = self.current_loc();
            return Err(ParseError::Limit {
                loc: loc.clone(),
                message: format!("block depth exceeds {MAX_BLOCK_DEPTH}"),
            });
        }
        self.directive_count += 1;
        if self.directive_count > MAX_DIRECTIVES {
            let loc = self.current_loc();
            return Err(ParseError::Limit {
                loc: loc.clone(),
                message: format!("directive count exceeds {MAX_DIRECTIVES}"),
            });
        }

        let name = self.consume_word().ok_or_else(|| ParseError::Syntax {
            loc: self.current_loc(),
            message: "expected directive name".into(),
        })?;
        let loc = self
            .tokens
            .get(self.pos.saturating_sub(1))
            .map(|t| t.loc.clone())
            .unwrap_or_else(|| SourceLoc::synthetic("input"));

        let args = self.parse_args()?;
        let block = if self.peek_lbrace() {
            Some(self.parse_block(depth + 1)?)
        } else {
            None
        };

        Ok(Directive {
            name,
            args,
            block,
            loc,
        })
    }

    fn parse_block(&mut self, depth: usize) -> Result<Block, ParseError> {
        self.expect_lbrace()?;
        let mut directives = Vec::new();
        while !self.eof() && !self.peek_rbrace() {
            directives.push(self.parse_directive(depth)?);
        }
        self.expect_rbrace()?;
        Ok(Block { directives })
    }

    fn parse_args(&mut self) -> Result<Vec<String>, ParseError> {
        let mut args = Vec::new();
        loop {
            match self.peek_kind() {
                Some(TokenKind::Semicolon) => {
                    self.next();
                    return Ok(args);
                }
                Some(TokenKind::LBrace) => return Ok(args),
                Some(TokenKind::RBrace) => {
                    return Err(ParseError::Syntax {
                        loc: self.current_loc(),
                        message: "expected `;` before `}`".into(),
                    });
                }
                Some(TokenKind::Word(_)) => {
                    if let Some(word) = self.consume_word() {
                        args.push(word);
                    }
                }
                None => {
                    return Err(ParseError::Syntax {
                        loc: self.current_loc(),
                        message: "unexpected end of file in directive".into(),
                    });
                }
            }
        }
    }

    fn expect_lbrace(&mut self) -> Result<(), ParseError> {
        match self.next() {
            Some(Token {
                kind: TokenKind::LBrace,
                ..
            }) => Ok(()),
            _ => Err(ParseError::Syntax {
                loc: self.current_loc(),
                message: "expected `{`".into(),
            }),
        }
    }

    fn expect_rbrace(&mut self) -> Result<(), ParseError> {
        match self.next() {
            Some(Token {
                kind: TokenKind::RBrace,
                ..
            }) => Ok(()),
            _ => Err(ParseError::Syntax {
                loc: self.current_loc(),
                message: "expected `}`".into(),
            }),
        }
    }

    fn consume_word(&mut self) -> Option<String> {
        match self.next() {
            Some(Token {
                kind: TokenKind::Word(w),
                ..
            }) => Some(w),
            _ => {
                self.pos = self.pos.saturating_sub(1);
                None
            }
        }
    }

    fn peek_kind(&self) -> Option<&TokenKind> {
        self.tokens.get(self.pos).map(|t| &t.kind)
    }

    fn peek_lbrace(&self) -> bool {
        matches!(self.peek_kind(), Some(TokenKind::LBrace))
    }

    fn peek_rbrace(&self) -> bool {
        matches!(self.peek_kind(), Some(TokenKind::RBrace))
    }

    fn next(&mut self) -> Option<Token> {
        if self.pos >= self.tokens.len() {
            return None;
        }
        let t = self.tokens[self.pos].clone();
        self.pos += 1;
        Some(t)
    }

    fn current_loc(&self) -> SourceLoc {
        self.tokens
            .get(self.pos)
            .map(|t| t.loc.clone())
            .unwrap_or_else(|| SourceLoc::synthetic("input"))
    }

    fn eof(&self) -> bool {
        self.pos >= self.tokens.len()
    }
}

pub fn parse_error_report(err: &ParseError, report: &mut CompatibilityReport) {
    let (loc, message) = match err {
        ParseError::Syntax { loc, message } | ParseError::Limit { loc, message } => {
            (loc.clone(), message.clone())
        }
    };
    report.push(&loc, "parse", CompatStatus::Error, message, None);
}
