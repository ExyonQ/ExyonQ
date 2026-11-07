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
//! OpenLiteSpeed `perClientConnLimit` → ExyonQ `[modules.ratelimit.client]`.
//!
//! This is deliberately a *fragment* importer, not a general OLS migration:
//! other directives are outside OL-03 and are not silently approximated. The
//! parser accepts a complete OLS configuration and extracts exactly one
//! `perClientConnLimit` block; zero or multiple blocks are rejected so a
//! server/vhost selection cannot be guessed.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// Result of translating one block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translation {
    /// TOML fragment to merge into an ExyonQ configuration.
    pub toml: String,
    /// OLS field names present in the source block (excluding defaults).
    pub fields: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslateError {
    message: String,
}

impl TranslateError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TranslateError {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    Open,
    Close,
}

fn tokenize(source: &str) -> Result<Vec<Token>, TranslateError> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    for ch in source.chars() {
        match ch {
            '#' => {
                if !word.is_empty() {
                    tokens.push(Token::Word(std::mem::take(&mut word)));
                }
                // Comment until newline: skip, then continue tokenizing.
                // Handled by the state below rather than a second pass.
                word.push('\u{0}');
            }
            '\n' => {
                if word.ends_with('\u{0}') {
                    word.clear();
                }
                if !word.is_empty() {
                    tokens.push(Token::Word(std::mem::take(&mut word)));
                }
            }
            '{' | '}' | ';' | '=' | '"' | '\'' => {
                if !word.is_empty() {
                    tokens.push(Token::Word(std::mem::take(&mut word)));
                }
                if word.ends_with('\u{0}') {
                    word.clear();
                }
                if ch == '{' {
                    tokens.push(Token::Open);
                } else if ch == '}' {
                    tokens.push(Token::Close);
                }
            }
            ch if ch.is_whitespace() => {
                if word.ends_with('\u{0}') {
                    word.clear();
                }
                if !word.is_empty() {
                    tokens.push(Token::Word(std::mem::take(&mut word)));
                }
            }
            '\u{0}' => {}
            ch => word.push(ch),
        }
    }
    if word.ends_with('\u{0}') {
        word.clear();
    }
    if !word.is_empty() {
        tokens.push(Token::Word(word));
    }
    Ok(tokens)
}

/// OpenLiteSpeed 1.9.4 parser defaults for an omitted field.
///
/// `HttpServerImpl::configSecurity` reads `softLimit`/`hardLimit` with
/// `getLongValue(pNode, name, 1, INT_MAX, INT_MAX)`, `gracePeriod` with a
/// default of `10` and `banPeriod` with a default of `60`. A missing limit is
/// therefore "no limit" (`INT_MAX`), not the WebAdmin form value.
const OLS_DEFAULT_LIMIT: u64 = i32::MAX as u64;
const OLS_DEFAULT_GRACE: u64 = 10;
const OLS_DEFAULT_BAN: u64 = 60;
const OLS_MAX_GRACE: u64 = 3600;

fn default_u64(field: &str) -> u64 {
    match field {
        "softLimit" | "hardLimit" => OLS_DEFAULT_LIMIT,
        "gracePeriod" => OLS_DEFAULT_GRACE,
        "banPeriod" => OLS_DEFAULT_BAN,
        _ => 0,
    }
}

/// Reject values OpenLiteSpeed's own parser range check would not accept.
fn check_range(field: &str, value: u64, max: u64) -> Result<(), TranslateError> {
    if value < 1 {
        return Err(TranslateError::new(format!(
            "{field} must be >= 1 (OpenLiteSpeed range 1..={max}), got {value}"
        )));
    }
    if value > max {
        return Err(TranslateError::new(format!(
            "{field} must be <= {max} (OpenLiteSpeed range 1..={max}), got {value}"
        )));
    }
    Ok(())
}

fn check_rate_range(field: &str, value: u64) -> Result<(), TranslateError> {
    if value > OLS_DEFAULT_LIMIT {
        return Err(TranslateError::new(format!(
            "{field} must be <= {} (OpenLiteSpeed range 0..={}), got {value}",
            OLS_DEFAULT_LIMIT, OLS_DEFAULT_LIMIT
        )));
    }
    Ok(())
}

fn parse_integer(field: &str, raw: &str) -> Result<u64, TranslateError> {
    raw.parse::<u64>().map_err(|_| {
        TranslateError::new(format!(
            "{field} must be a non-negative integer, got `{raw}`"
        ))
    })
}

fn checked_field(field: &str) -> Result<(), TranslateError> {
    const KNOWN: [&str; 9] = [
        "staticReqPerSec",
        "dynReqPerSec",
        "outBandwidth",
        "inBandwidth",
        "softLimit",
        "hardLimit",
        "gracePeriod",
        "banPeriod",
        "blockBadReq",
    ];
    if KNOWN.contains(&field) {
        Ok(())
    } else {
        Err(TranslateError::new(format!(
            "unsupported perClientConnLimit field `{field}`; refusing to ignore it"
        )))
    }
}

/// Round up to the granularity OpenLiteSpeed applies.
fn round_up(value: u64, quantum: u64) -> u64 {
    value.div_ceil(quantum) * quantum
}

/// Translate the single `perClientConnLimit` block in `source`.
pub fn translate(source: &str) -> Result<Translation, TranslateError> {
    let tokens = tokenize(source)?;
    let starts = tokens
        .iter()
        .enumerate()
        .filter_map(|(idx, token)| match token {
            Token::Word(word) if word == "perClientConnLimit" => Some(idx),
            _ => None,
        })
        .collect::<Vec<_>>();
    match starts.as_slice() {
        [] => Err(TranslateError::new(
            "no perClientConnLimit block found; refusing an empty migration",
        )),
        [_, _, ..] => Err(TranslateError::new(
            "multiple perClientConnLimit blocks found; migrate one server/vhost at a time",
        )),
        [start] => {
            if tokens.get(start + 1) != Some(&Token::Open) {
                return Err(TranslateError::new(
                    "perClientConnLimit must be followed by `{`",
                ));
            }
            let mut raw: BTreeMap<&'static str, u64> = BTreeMap::new();
            let mut idx = start + 2;
            loop {
                match tokens.get(idx) {
                    Some(Token::Close) => break,
                    Some(Token::Word(field)) => {
                        checked_field(field)?;
                        let value = match tokens.get(idx + 1) {
                            Some(Token::Word(value)) => value,
                            _ => {
                                return Err(TranslateError::new(format!(
                                    "perClientConnLimit field `{field}` needs a value"
                                )))
                            }
                        };
                        let value = parse_integer(field, value)?;
                        let key = match field.as_str() {
                            "staticReqPerSec" => "staticReqPerSec",
                            "dynReqPerSec" => "dynReqPerSec",
                            "outBandwidth" => "outBandwidth",
                            "inBandwidth" => "inBandwidth",
                            "softLimit" => "softLimit",
                            "hardLimit" => "hardLimit",
                            "gracePeriod" => "gracePeriod",
                            "banPeriod" => "banPeriod",
                            "blockBadReq" => "blockBadReq",
                            _ => unreachable!("checked_field accepted an unknown field"),
                        };
                        if raw.insert(key, value).is_some() {
                            return Err(TranslateError::new(format!(
                                "duplicate perClientConnLimit field `{field}`"
                            )));
                        }
                        idx += 2;
                    }
                    Some(Token::Open) => {
                        return Err(TranslateError::new(
                            "nested blocks inside perClientConnLimit are not supported",
                        ))
                    }
                    None => break,
                }
            }

            let mut fields = raw.keys().copied().collect::<Vec<_>>();
            fields.sort_unstable();
            let block_bad_req = raw.get("blockBadReq").copied().unwrap_or(0) != 0;
            if raw.get("blockBadReq").is_some_and(|value| *value > 1) {
                return Err(TranslateError::new("blockBadReq must be 0 or 1"));
            }

            let out_raw = raw.get("outBandwidth").copied().unwrap_or(0);
            let out = (out_raw > 0).then(|| round_up(out_raw, 4096));
            let in_raw = raw.get("inBandwidth").copied();
            let inbound = match in_raw {
                Some(0) => None,
                Some(value) => Some(round_up(value, 1024)),
                // OpenLiteSpeed defaults an omitted inBandwidth to outBandwidth.
                None => out.map(|value| round_up(value, 1024)),
            };
            let static_rps = raw.get("staticReqPerSec").copied().unwrap_or(0);
            let dynamic_rps = raw.get("dynReqPerSec").copied().unwrap_or(0);
            let soft = raw
                .get("softLimit")
                .copied()
                .unwrap_or_else(|| default_u64("softLimit"));
            let hard = raw
                .get("hardLimit")
                .copied()
                .unwrap_or_else(|| default_u64("hardLimit"));
            let grace = raw
                .get("gracePeriod")
                .copied()
                .unwrap_or_else(|| default_u64("gracePeriod"));
            let ban = raw
                .get("banPeriod")
                .copied()
                .unwrap_or_else(|| default_u64("banPeriod"));
            check_range("softLimit", soft, OLS_DEFAULT_LIMIT)?;
            check_range("hardLimit", hard, OLS_DEFAULT_LIMIT)?;
            check_range("gracePeriod", grace, OLS_MAX_GRACE)?;
            check_range("banPeriod", ban, OLS_DEFAULT_LIMIT)?;
            check_rate_range("outBandwidth", out_raw)?;
            check_rate_range("inBandwidth", in_raw.unwrap_or(0))?;
            check_rate_range("staticReqPerSec", static_rps)?;
            check_rate_range("dynReqPerSec", dynamic_rps)?;
            // OpenLiteSpeed accepts soft > hard and lets the hard check win.
            // Preserve the source pair instead of rejecting a valid OLS block;
            // ExyonQ checks the hard ceiling first, so the runtime is the same.

            let mut out_toml = String::from(
                "# Generated by exyonq-compat ols-throttle from OpenLiteSpeed perClientConnLimit.\n\
                 # Merge this fragment into a complete ExyonQ configuration.\n\
                 [modules.ratelimit.client]\n\
                 enabled = true\n",
            );
            if soft > 0 {
                out_toml.push_str(&format!("soft_connections_per_ip = {soft}\n"));
            }
            if hard > 0 {
                out_toml.push_str(&format!("max_connections_per_ip = {hard}\n"));
            }
            if let Some(value) = out {
                out_toml.push_str(&format!("bandwidth_bytes_per_second = {value}\n"));
            }
            if let Some(value) = inbound {
                out_toml.push_str(&format!("input_bandwidth_bytes_per_second = {value}\n"));
            }
            if static_rps > 0 {
                out_toml.push_str(&format!("static_requests_per_second = {static_rps}\n"));
            }
            if dynamic_rps > 0 {
                out_toml.push_str(&format!("dynamic_requests_per_second = {dynamic_rps}\n"));
            }
            if block_bad_req {
                out_toml.push_str("block_bad_req = true\n");
            }
            if ban > 0 {
                out_toml.push_str(&format!("ban_seconds = {ban}\n"));
            }
            if grace > 0 {
                out_toml.push_str(&format!("grace_seconds = {grace}\n"));
            }
            Ok(Translation {
                toml: out_toml,
                fields,
            })
        }
    }
}

/// Read and translate a file.
pub fn translate_file(path: &Path) -> Result<Translation, TranslateError> {
    let source = std::fs::read_to_string(path)
        .map_err(|err| TranslateError::new(format!("cannot read {}: {err}", path.display())))?;
    translate(&source)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: &str = r#"
        # OLS httpd_config.conf
        perClientConnLimit {
            staticReqPerSec      2
            dynReqPerSec         1
            outBandwidth         100000
            inBandwidth          5000
            softLimit            2
            hardLimit            3
            gracePeriod          1
            banPeriod            2
            blockBadReq          1
        }
    "#;

    #[test]
    fn translates_every_field_and_rounds_bandwidth() {
        let translation = translate(BLOCK).expect("translation");
        assert!(translation.toml.contains("static_requests_per_second = 2"));
        assert!(translation.toml.contains("dynamic_requests_per_second = 1"));
        assert!(translation
            .toml
            .contains("bandwidth_bytes_per_second = 102400"));
        assert!(translation
            .toml
            .contains("input_bandwidth_bytes_per_second = 5120"));
        assert!(translation.toml.contains("soft_connections_per_ip = 2"));
        assert!(translation.toml.contains("max_connections_per_ip = 3"));
        assert!(translation.toml.contains("grace_seconds = 1"));
        assert!(translation.toml.contains("ban_seconds = 2"));
        assert!(translation.toml.contains("block_bad_req = true"));
        assert_eq!(translation.fields.len(), 9);
    }

    #[test]
    fn missing_in_bandwidth_follows_out_bandwidth() {
        let translation =
            translate("perClientConnLimit { outBandwidth 8192 }").expect("translation");
        assert!(translation
            .toml
            .contains("bandwidth_bytes_per_second = 8192"));
        assert!(translation
            .toml
            .contains("input_bandwidth_bytes_per_second = 8192"));
    }

    #[test]
    fn explicit_zero_in_bandwidth_disables_inbound() {
        let translation =
            translate("perClientConnLimit { outBandwidth 8192 inBandwidth 0 blockBadReq 0 }")
                .expect("translation");
        assert!(!translation
            .toml
            .contains("input_bandwidth_bytes_per_second"));
    }

    #[test]
    fn absent_fields_use_openlitespeed_parser_defaults() {
        let translation =
            translate("perClientConnLimit { outBandwidth 8192 }").expect("translation");
        assert!(translation
            .toml
            .contains("soft_connections_per_ip = 2147483647"));
        assert!(translation
            .toml
            .contains("max_connections_per_ip = 2147483647"));
        assert!(translation.toml.contains("grace_seconds = 10"));
        assert!(translation.toml.contains("ban_seconds = 60"));
    }

    #[test]
    fn soft_limit_without_hard_limit_uses_int_max_hard_limit() {
        let translation = translate("perClientConnLimit { softLimit 2 }").expect("translation");
        assert!(translation.toml.contains("soft_connections_per_ip = 2"));
        assert!(translation
            .toml
            .contains("max_connections_per_ip = 2147483647"));
    }

    #[test]
    fn hard_limit_without_soft_limit_uses_int_max_soft_limit() {
        let translation = translate("perClientConnLimit { hardLimit 3 }").expect("translation");
        assert!(translation
            .toml
            .contains("soft_connections_per_ip = 2147483647"));
        assert!(translation.toml.contains("max_connections_per_ip = 3"));
    }

    #[test]
    fn soft_limit_above_hard_limit_is_preserved() {
        let translation =
            translate("perClientConnLimit { softLimit 3 hardLimit 2 }").expect("translation");
        assert!(translation.toml.contains("soft_connections_per_ip = 3"));
        assert!(translation.toml.contains("max_connections_per_ip = 2"));
    }

    #[test]
    fn oversized_request_and_bandwidth_fields_fail_closed() {
        for source in [
            "perClientConnLimit { staticReqPerSec 2147483648 }",
            "perClientConnLimit { dynReqPerSec 2147483648 }",
            "perClientConnLimit { outBandwidth 2147483648 }",
            "perClientConnLimit { inBandwidth 2147483648 }",
        ] {
            assert!(translate(source).is_err(), "{source} should be rejected");
        }
    }

    #[test]
    fn out_of_range_values_fail_closed() {
        for source in [
            "perClientConnLimit { softLimit 0 }",
            "perClientConnLimit { hardLimit 0 }",
            "perClientConnLimit { gracePeriod 0 }",
            "perClientConnLimit { gracePeriod 3601 }",
            "perClientConnLimit { banPeriod 0 }",
        ] {
            assert!(translate(source).is_err(), "{source} should be rejected");
        }
    }

    #[test]
    fn unknown_field_fails_closed() {
        let err = translate("perClientConnLimit { outBandwidth 8192 mystery 1 }").unwrap_err();
        assert!(err.to_string().contains("mystery"));
    }

    #[test]
    fn multiple_blocks_fail_closed() {
        let err = translate(
            "perClientConnLimit { outBandwidth 8192 }\nperClientConnLimit { outBandwidth 4096 }",
        )
        .unwrap_err();
        assert!(err.to_string().contains("multiple"));
    }

    #[test]
    fn missing_block_fails_closed() {
        assert!(translate("server { port 8088 }").is_err());
    }

    #[test]
    fn bad_boolean_fails_closed() {
        assert!(translate("perClientConnLimit { blockBadReq 2 }").is_err());
    }
}
