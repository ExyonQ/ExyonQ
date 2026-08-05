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
use thiserror::Error;

use exyonq_config_ir::{Diagnostic, DiagnosticCode};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("empty input")]
    Empty,
    #[error("expected listener header like `:8080` or `host:443`")]
    MissingListener,
    #[error("unclosed block")]
    UnclosedBlock,
    #[error("unexpected token: {0}")]
    UnexpectedToken(String),
    #[error("route block requires `match` path")]
    MissingRoutePath,
}

impl ParseError {
    pub fn to_diagnostic(&self) -> Diagnostic {
        Diagnostic::error(DiagnosticCode::SurfaceParseError, self.to_string())
            .with_documentation("docs/config/diagnostic-codes.md#exy-config-0009")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerfileAst {
    pub listener: String,
    pub routes: Vec<RouteBlock>,
    pub metrics: bool,
    pub compression: bool,
    pub ratelimit: Option<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteBlock {
    pub path: String,
    pub proxy: Option<ProxyDirective>,
    pub root: Option<String>,
    pub index: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyDirective {
    pub upstream_name: String,
    pub port: u16,
    pub timeout_ms: Option<u64>,
}

pub fn parse_serverfile(input: &str) -> Result<ServerfileAst, ParseError> {
    let tokens = tokenize(input)?;
    parse_tokens(&tokens)
}

fn tokenize(input: &str) -> Result<Vec<Token>, ParseError> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        if ch == '#' {
            while chars.next().is_some_and(|c| c != '\n') {}
            continue;
        }
        if ch == '{' {
            chars.next();
            tokens.push(Token::LBrace);
            continue;
        }
        if ch == '}' {
            chars.next();
            tokens.push(Token::RBrace);
            continue;
        }
        let mut word = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == '{' || c == '}' {
                break;
            }
            word.push(c);
            chars.next();
        }
        if word.is_empty() {
            return Err(ParseError::UnexpectedToken(String::new()));
        }
        tokens.push(Token::Word(word));
    }
    Ok(tokens)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    LBrace,
    RBrace,
    Word(String),
}

fn parse_tokens(tokens: &[Token]) -> Result<ServerfileAst, ParseError> {
    if tokens.is_empty() {
        return Err(ParseError::Empty);
    }
    let listener = match tokens.first() {
        Some(Token::Word(w)) => w.clone(),
        _ => return Err(ParseError::MissingListener),
    };
    if !listener.starts_with(':') && !listener.contains(':') {
        return Err(ParseError::MissingListener);
    }
    let mut idx = 1;
    expect(tokens, &mut idx, Token::LBrace)?;
    let mut routes = Vec::new();
    let mut metrics = false;
    let mut compression = false;
    let mut ratelimit = None;
    while idx < tokens.len() {
        match &tokens[idx] {
            Token::RBrace => {
                idx += 1;
                break;
            }
            Token::Word(word) if word == "route" => {
                idx += 1;
                routes.push(parse_route(tokens, &mut idx)?);
            }
            Token::Word(word) if word == "metrics" => {
                idx += 1;
                let val = expect_word(tokens, &mut idx)?;
                metrics = val == "on";
            }
            Token::Word(word) if word == "compression" => {
                idx += 1;
                let val = expect_word(tokens, &mut idx)?;
                compression = val == "on" || val == "gzip";
            }
            Token::Word(word) if word == "ratelimit" => {
                idx += 1;
                ratelimit = Some(parse_ratelimit_tokens(tokens, &mut idx)?);
            }
            Token::Word(other) => {
                return Err(ParseError::UnexpectedToken(other.clone()));
            }
            _ => return Err(ParseError::UnexpectedToken(format!("{:?}", tokens[idx]))),
        }
    }
    if idx != tokens.len() {
        return Err(ParseError::UnexpectedToken(
            "trailing tokens after server block".into(),
        ));
    }
    Ok(ServerfileAst {
        listener,
        routes,
        metrics,
        compression,
        ratelimit,
    })
}

fn parse_route(tokens: &[Token], idx: &mut usize) -> Result<RouteBlock, ParseError> {
    let path = expect_word(tokens, idx)?;
    if !path.starts_with('/') {
        return Err(ParseError::MissingRoutePath);
    }
    expect(tokens, idx, Token::LBrace)?;
    let mut proxy = None;
    let mut root = None;
    let mut index = None;
    loop {
        match &tokens[*idx] {
            Token::RBrace => {
                *idx += 1;
                break;
            }
            Token::Word(word) if word == "proxy" => {
                *idx += 1;
                let upstream_name = expect_word(tokens, idx)?;
                let port: u16 = expect_word(tokens, idx)?
                    .parse()
                    .map_err(|_| ParseError::UnexpectedToken("port".into()))?;
                let mut timeout_ms = None;
                if *idx < tokens.len() {
                    if let Token::Word(w) = &tokens[*idx] {
                        if w == "timeout" {
                            *idx += 1;
                            let raw = expect_word(tokens, idx)?;
                            timeout_ms = Some(parse_duration_ms(&raw)?);
                        }
                    }
                }
                proxy = Some(ProxyDirective {
                    upstream_name,
                    port,
                    timeout_ms,
                });
            }
            Token::Word(word) if word == "root" => {
                *idx += 1;
                root = Some(expect_word(tokens, idx)?);
            }
            Token::Word(word) if word == "index" => {
                *idx += 1;
                index = Some(expect_word(tokens, idx)?);
            }
            Token::Word(other) => return Err(ParseError::UnexpectedToken(other.clone())),
            _ => return Err(ParseError::UnexpectedToken("route inner".into())),
        }
    }
    Ok(RouteBlock {
        path,
        proxy,
        root,
        index,
    })
}

fn parse_ratelimit_tokens(tokens: &[Token], idx: &mut usize) -> Result<(u32, u32), ParseError> {
    let spec = expect_word(tokens, idx)?;
    let rps = parse_rps_spec(&spec)?;
    let burst = if *idx < tokens.len() {
        if let Token::Word(word) = &tokens[*idx] {
            if word == "burst" {
                *idx += 1;
                let raw = expect_word(tokens, idx)?;
                raw.parse().map_err(|_| ParseError::UnexpectedToken(raw))?
            } else {
                rps.saturating_mul(2)
            }
        } else {
            rps.saturating_mul(2)
        }
    } else {
        rps.saturating_mul(2)
    };
    Ok((rps, burst))
}

fn parse_rps_spec(spec: &str) -> Result<u32, ParseError> {
    let digits: String = spec.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits
        .parse()
        .map_err(|_| ParseError::UnexpectedToken(spec.into()))
}

fn parse_duration_ms(raw: &str) -> Result<u64, ParseError> {
    if raw.ends_with("ms") {
        raw.trim_end_matches("ms")
            .parse()
            .map_err(|_| ParseError::UnexpectedToken(raw.into()))
    } else if raw.ends_with('s') {
        let secs: u64 = raw
            .trim_end_matches('s')
            .parse()
            .map_err(|_| ParseError::UnexpectedToken(raw.into()))?;
        Ok(secs * 1000)
    } else {
        Err(ParseError::UnexpectedToken(raw.into()))
    }
}

fn expect(tokens: &[Token], idx: &mut usize, expected: Token) -> Result<(), ParseError> {
    if *idx >= tokens.len() {
        return Err(ParseError::UnclosedBlock);
    }
    if std::mem::discriminant(&tokens[*idx]) == std::mem::discriminant(&expected) {
        *idx += 1;
        Ok(())
    } else {
        Err(ParseError::UnexpectedToken(format!(
            "expected {expected:?}"
        )))
    }
}

fn expect_word(tokens: &[Token], idx: &mut usize) -> Result<String, ParseError> {
    if *idx >= tokens.len() {
        return Err(ParseError::UnclosedBlock);
    }
    match &tokens[*idx] {
        Token::Word(w) => {
            *idx += 1;
            Ok(w.clone())
        }
        _ => Err(ParseError::UnexpectedToken("word".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_serverfile() {
        let ast = parse_serverfile(
            r#"
:8080 {
    route /api {
        proxy backend 9000
    }
}
"#,
        )
        .unwrap();
        assert_eq!(ast.listener, ":8080");
        assert_eq!(ast.routes.len(), 1);
    }

    #[test]
    fn parses_ratelimit_with_burst() {
        let ast = parse_serverfile(
            r#"
:8080 {
    route /api { proxy backend 9000 }
    ratelimit 5000r/s burst 5000
}
"#,
        )
        .unwrap();
        assert_eq!(ast.ratelimit, Some((5000, 5000)));
    }

    #[test]
    fn fmt_roundtrip_with_ratelimit() {
        let input =
            ":8080 {\n    route /api { proxy backend 9000 }\n    ratelimit 5000r/s burst 5000\n}\n";
        let formatted = crate::fmt_serverfile(input).unwrap();
        let reparsed = parse_serverfile(&formatted).unwrap();
        assert_eq!(reparsed.ratelimit, Some((5000, 5000)));
    }
}
