//! Minimal `.htaccess` line parser.

use crate::ast::{ParsedDirective, ParsedFile};
use crate::limits::MAX_DIRECTIVES_PER_FILE;
use crate::report::SourceLoc;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("directive limit exceeded ({MAX_DIRECTIVES_PER_FILE})")]
    LimitDirectives,
    #[error("{0}")]
    Message(String),
}

pub fn parse_htaccess(path: &str, content: &str) -> Result<ParsedFile, ParseError> {
    let mut directives = Vec::new();
    for (line_no, raw_line) in content.lines().enumerate() {
        let line = strip_comment(raw_line);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('<') {
            continue;
        }
        if directives.len() >= MAX_DIRECTIVES_PER_FILE {
            return Err(ParseError::LimitDirectives);
        }
        let tokens = tokenize(line)?;
        if tokens.is_empty() {
            continue;
        }
        let name = tokens[0].to_ascii_lowercase();
        directives.push(ParsedDirective {
            name,
            args: tokens[1..].to_vec(),
            loc: SourceLoc {
                file: path.to_string(),
                line: (line_no + 1) as u32,
                column: 1,
            },
        });
    }
    Ok(ParsedFile {
        path: path.to_string(),
        directives,
    })
}

fn strip_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let mut prev_escape = false;
    for (i, ch) in line.char_indices() {
        if ch == '\'' && !in_double && !prev_escape {
            in_single = !in_single;
        } else if ch == '"' && !in_single && !prev_escape {
            in_double = !in_double;
        } else if ch == '#' && !in_single && !in_double {
            return &line[..i];
        }
        prev_escape = ch == '\\' && !prev_escape;
        if ch != '\\' {
            prev_escape = false;
        }
    }
    line
}

fn tokenize(line: &str) -> Result<Vec<String>, ParseError> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if in_double || in_single => {
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            }
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if !cur.is_empty() {
                    out.push(cur.clone());
                    cur.clear();
                }
            }
            _ => cur.push(ch),
        }
    }
    if in_single || in_double {
        return Err(ParseError::Message("unclosed quote".into()));
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}
