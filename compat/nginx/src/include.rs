//! Local `include` resolution with safety limits.

use crate::lexer::{lex, LexError};
use crate::limits::{MAX_INCLUDED_FILES, MAX_INCLUDE_DEPTH, MAX_TOTAL_INPUT_BYTES};
use crate::parser::{parse_error_report, ParseError, Parser};
use crate::report::{CompatStatus, CompatibilityReport, SourceLoc};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IncludeError {
    #[error("{file}:{line}:{column}: {message}")]
    Diagnostic {
        file: String,
        line: u32,
        column: u32,
        message: String,
    },
    #[error("lex error: {0}")]
    Lex(#[from] LexError),
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct IncludeLoader {
    pub entry_root: PathBuf,
    seen: HashSet<PathBuf>,
    files_loaded: usize,
    bytes_loaded: usize,
}

impl IncludeLoader {
    pub fn new(entry: &Path) -> Self {
        let entry_root = entry
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            entry_root: entry_root.canonicalize().unwrap_or(entry_root),
            seen: HashSet::new(),
            files_loaded: 0,
            bytes_loaded: 0,
        }
    }

    pub fn load_file(
        &mut self,
        path: &Path,
        depth: usize,
        report: &mut CompatibilityReport,
    ) -> Result<Vec<crate::lexer::Token>, IncludeError> {
        if depth > MAX_INCLUDE_DEPTH {
            return Err(IncludeError::Diagnostic {
                file: path.display().to_string(),
                line: 1,
                column: 1,
                message: format!("include depth exceeds {MAX_INCLUDE_DEPTH}"),
            });
        }
        if self.files_loaded >= MAX_INCLUDED_FILES {
            return Err(IncludeError::Diagnostic {
                file: path.display().to_string(),
                line: 1,
                column: 1,
                message: format!("included file count exceeds {MAX_INCLUDED_FILES}"),
            });
        }

        let canonical = path.canonicalize().map_err(|e| IncludeError::Diagnostic {
            file: path.display().to_string(),
            line: 1,
            column: 1,
            message: format!("cannot resolve path: {e}"),
        })?;

        if !canonical.starts_with(&self.entry_root) {
            return Err(IncludeError::Diagnostic {
                file: path.display().to_string(),
                line: 1,
                column: 1,
                message: "include path escapes allowed root".into(),
            });
        }

        if !self.seen.insert(canonical.clone()) {
            return Err(IncludeError::Diagnostic {
                file: path.display().to_string(),
                line: 1,
                column: 1,
                message: "include cycle detected".into(),
            });
        }

        let content = std::fs::read_to_string(&canonical)?;
        self.bytes_loaded += content.len();
        if self.bytes_loaded > MAX_TOTAL_INPUT_BYTES {
            return Err(IncludeError::Diagnostic {
                file: canonical.display().to_string(),
                line: 1,
                column: 1,
                message: format!("total input exceeds {MAX_TOTAL_INPUT_BYTES} bytes"),
            });
        }
        self.files_loaded += 1;

        let file_label = canonical.display().to_string();
        let mut tokens = lex(&file_label, &content)?;
        self.expand_includes(&canonical, &mut tokens, depth + 1, report)?;
        Ok(tokens)
    }

    fn expand_includes(
        &mut self,
        parent: &Path,
        tokens: &mut Vec<crate::lexer::Token>,
        depth: usize,
        report: &mut CompatibilityReport,
    ) -> Result<(), IncludeError> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < tokens.len() {
            if let crate::lexer::TokenKind::Word(name) = &tokens[i].kind {
                if name == "include" {
                    let loc = tokens[i].loc.clone();
                    i += 1;
                    let mut pattern = String::new();
                    while i < tokens.len() {
                        match &tokens[i].kind {
                            crate::lexer::TokenKind::Word(w) => {
                                if pattern.is_empty() {
                                    pattern = w.clone();
                                } else {
                                    pattern.push(' ');
                                    pattern.push_str(w);
                                }
                                i += 1;
                            }
                            crate::lexer::TokenKind::Semicolon => {
                                i += 1;
                                break;
                            }
                            _ => break,
                        }
                    }
                    self.resolve_include(parent, &pattern, &loc, depth, report, &mut out)?;
                    continue;
                }
            }
            out.push(tokens[i].clone());
            i += 1;
        }
        *tokens = out;
        Ok(())
    }

    fn resolve_include(
        &mut self,
        parent: &Path,
        pattern: &str,
        loc: &SourceLoc,
        depth: usize,
        report: &mut CompatibilityReport,
        out: &mut Vec<crate::lexer::Token>,
    ) -> Result<(), IncludeError> {
        let base = parent.parent().unwrap_or(parent);
        if pattern.ends_with("mime.types") || pattern.contains("mime.types") {
            report.push(
                loc,
                "include",
                CompatStatus::Ignored,
                format!("known types file `{pattern}` ignored for IR import"),
                None,
            );
            return Ok(());
        }

        let paths = expand_glob(base, pattern)?;
        if paths.is_empty() {
            report.push(
                loc,
                "include",
                CompatStatus::Partial,
                format!("include pattern `{pattern}` matched no files"),
                None,
            );
            return Ok(());
        }

        report.push(
            loc,
            "include",
            CompatStatus::Supported,
            format!("expanded include `{pattern}` ({} file(s))", paths.len()),
            None,
        );

        for path in paths {
            let child_tokens = self.load_file(&path, depth, report)?;
            out.extend(child_tokens);
        }
        Ok(())
    }
}

fn expand_glob(base: &Path, pattern: &str) -> Result<Vec<PathBuf>, IncludeError> {
    let pattern_path = PathBuf::from(pattern);
    let (dir, glob_part) = if pattern.contains('*') {
        let parent = pattern_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let file = pattern_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| pattern.to_string());
        (base.join(parent), file)
    } else {
        (base.to_path_buf(), pattern.to_string())
    };

    let dir = dir.canonicalize().unwrap_or(dir);
    if !glob_part.contains('*') {
        let single = safe_join(&dir, Path::new(&glob_part))?;
        return Ok(vec![single]);
    }

    let prefix = glob_part.split('*').next().unwrap_or("");
    let suffix = glob_part.rsplit('*').next().unwrap_or("");
    let mut matches = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(prefix) && name.ends_with(suffix) {
            matches.push(entry.path());
        }
    }
    matches.sort();
    Ok(matches)
}

fn safe_join(base: &Path, rel: &Path) -> Result<PathBuf, IncludeError> {
    for comp in rel.components() {
        match comp {
            Component::ParentDir => {
                return Err(IncludeError::Diagnostic {
                    file: base.display().to_string(),
                    line: 1,
                    column: 1,
                    message: "path traversal in include".into(),
                });
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(IncludeError::Diagnostic {
                    file: base.display().to_string(),
                    line: 1,
                    column: 1,
                    message: "absolute include path not allowed".into(),
                });
            }
            Component::Normal(part) => {
                let joined = base.join(part);
                return joined.canonicalize().map_err(|e| IncludeError::Diagnostic {
                    file: base.display().to_string(),
                    line: 1,
                    column: 1,
                    message: format!("invalid include path: {e}"),
                });
            }
            Component::CurDir => {}
        }
    }
    Ok(base.to_path_buf())
}

pub fn load_config(
    path: &Path,
    report: &mut CompatibilityReport,
) -> Result<Vec<crate::lexer::Token>, IncludeError> {
    let mut loader = IncludeLoader::new(path);
    loader.load_file(path, 0, report)
}

pub fn load_config_string(
    label: &str,
    input: &str,
    _report: &mut CompatibilityReport,
) -> Result<Vec<crate::lexer::Token>, IncludeError> {
    if input.len() > MAX_TOTAL_INPUT_BYTES {
        return Err(IncludeError::Diagnostic {
            file: label.into(),
            line: 1,
            column: 1,
            message: format!("input exceeds {MAX_TOTAL_INPUT_BYTES} bytes"),
        });
    }
    let tokens = lex(label, input)?;
    Ok(tokens)
}

pub fn parse_tokens(
    tokens: &[crate::lexer::Token],
    report: &mut CompatibilityReport,
) -> Result<crate::ast::Config, IncludeError> {
    match Parser::parse(tokens, report) {
        Ok(cfg) => Ok(cfg),
        Err(e) => {
            parse_error_report(&e, report);
            Err(IncludeError::Parse(e))
        }
    }
}
