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
//! Budgeted normalization for WAF matching.

use exyonq_waf_api::WafTransform;
use std::borrow::Cow;

/// Limits how aggressively we decode before matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizeBudget {
    /// Max successful URL-decode rounds (default 2).
    pub max_url_decode_rounds: u8,
}

impl Default for NormalizeBudget {
    fn default() -> Self {
        Self {
            max_url_decode_rounds: 2,
        }
    }
}

/// Normalized view plus transforms applied (for evidence).
#[derive(Debug, Clone)]
pub struct NormalizedText<'a> {
    pub text: Cow<'a, str>,
    pub transforms: Vec<WafTransform>,
}

/// URL-decode up to `budget.max_url_decode_rounds`, recording evidence.
pub fn normalize_text<'a>(input: &'a str, budget: NormalizeBudget) -> NormalizedText<'a> {
    let mut transforms = Vec::new();
    if budget.max_url_decode_rounds == 0 || !input.as_bytes().contains(&b'%') {
        return NormalizedText {
            text: Cow::Borrowed(input),
            transforms,
        };
    }

    let mut current = input.to_string();
    let mut rounds = 0u8;
    while rounds < budget.max_url_decode_rounds {
        match percent_decode_once(&current) {
            Some(next) if next != current => {
                current = next;
                rounds += 1;
            }
            _ => break,
        }
    }
    if rounds > 0 {
        transforms.push(WafTransform::UrlDecode { rounds });
    }
    NormalizedText {
        text: Cow::Owned(current),
        transforms,
    }
}

/// Decode one pass of `%HH` (and `+` → space). Returns `None` if nothing changed.
fn percent_decode_once(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    if !bytes.contains(&b'%') && !bytes.contains(&b'+') {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut changed = false;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                changed = true;
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(h), Some(l)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                    out.push((h << 4) | l);
                    changed = true;
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    if !changed {
        return None;
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_encode_within_budget() {
        // %252e%252e%252f → %2e%2e%2f → ../
        let n = normalize_text("%252e%252e%252f", NormalizeBudget::default());
        assert_eq!(n.text.as_ref(), "../");
        assert_eq!(n.transforms, vec![WafTransform::UrlDecode { rounds: 2 }]);
    }

    #[test]
    fn budget_one_round_stops_early() {
        let n = normalize_text(
            "%252e%252e%252f",
            NormalizeBudget {
                max_url_decode_rounds: 1,
            },
        );
        assert_eq!(n.text.as_ref(), "%2e%2e%2f");
        assert_eq!(n.transforms, vec![WafTransform::UrlDecode { rounds: 1 }]);
    }
}
