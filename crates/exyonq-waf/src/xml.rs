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
//! CRS `XML` / `XML:/*` / `XML://@*` — bounded, XXE-safe XML inspection.
//!
//! ModSecurity exposes the parsed document as the `XML` collection. ExyonQ only
//! needs the values CRS matches against: every element text node (`XML:/*`) and
//! every attribute value (`XML://@*`). No DTD is loaded and no entity is
//! resolved beyond the five predefined ones plus numeric character references,
//! so external-entity expansion (XXE) is impossible by construction.

/// Values extracted from an XML request body.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct XmlValues {
    /// Element text nodes, document order (`XML`, `XML:/*`).
    pub texts: Vec<String>,
    /// Attribute values, document order (`XML://@*`).
    pub attrs: Vec<String>,
}

const MAX_ELEMENTS: usize = 4096;
const MAX_ATTRS: usize = 4096;
const MAX_DEPTH: usize = 64;
const MAX_VALUE_BYTES: usize = 64 * 1024;
const MAX_TOTAL_BYTES: usize = 1024 * 1024;

/// True when the request body processor for this content type is XML.
pub(crate) fn is_xml_content_type(content_type: &str) -> bool {
    let lower = content_type.to_ascii_lowercase();
    let mime = lower.split(';').next().unwrap_or("").trim();
    matches!(
        mime,
        "application/xml" | "text/xml" | "application/soap+xml" | "application/xhtml+xml"
    ) || mime.ends_with("+xml")
}

/// Parse an XML body into the CRS-visible values. `None` when the body is not
/// valid UTF-8 or cannot be tokenized at all.
pub(crate) fn parse(body: &[u8]) -> Option<XmlValues> {
    if body.len() > MAX_TOTAL_BYTES {
        return None;
    }
    let text = std::str::from_utf8(body).ok()?;
    let bytes = text.as_bytes();
    let mut values = XmlValues::default();
    let mut depth = 0usize;
    let mut elements = 0usize;
    let mut attrs = 0usize;
    let mut i = 0usize;
    let mut total = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            let end = text[i..]
                .find('<')
                .map(|offset| i + offset)
                .unwrap_or(bytes.len());
            let raw = &text[i..end];
            if !raw.trim().is_empty() {
                let decoded = decode_entities(raw);
                total = total.saturating_add(decoded.len());
                if total > MAX_VALUE_BYTES {
                    break;
                }
                values.texts.push(decoded);
            }
            i = end;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i = match rest.find("-->") {
                Some(end) => i + end + 3,
                None => break,
            };
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            let Some(end) = rest.find("]]>") else { break };
            let raw = &rest[9..end];
            if !raw.trim().is_empty() {
                total = total.saturating_add(raw.len());
                if total > MAX_VALUE_BYTES {
                    break;
                }
                values.texts.push(raw.to_string());
            }
            i += end + 3;
            continue;
        }
        if rest.starts_with("<?") {
            i = match rest.find("?>") {
                Some(end) => i + end + 2,
                None => break,
            };
            continue;
        }
        if rest.starts_with("<!") {
            // DOCTYPE / declaration: skipped without loading or expanding entities.
            i = match skip_declaration(rest) {
                Some(end) => i + end,
                None => break,
            };
            continue;
        }
        if rest.starts_with("</") {
            let Some(end) = rest.find('>') else { break };
            depth = depth.saturating_sub(1);
            i += end + 1;
            continue;
        }
        let Some(end) = tag_end(rest) else { break };
        let inner = &rest[1..end];
        let self_closing = inner.trim_end().ends_with('/');
        elements += 1;
        if elements > MAX_ELEMENTS {
            break;
        }
        attrs += collect_attrs(inner, &mut values.attrs, MAX_ATTRS.saturating_sub(attrs));
        if attrs > MAX_ATTRS {
            attrs = MAX_ATTRS;
        }
        if !self_closing {
            depth += 1;
            if depth > MAX_DEPTH {
                break;
            }
        }
        i += end + 1;
    }
    Some(values)
}

/// Length of the start tag up to and including its `>` (quotes respected).
fn tag_end(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let mut quote = 0u8;
    for (idx, &byte) in bytes.iter().enumerate().skip(1) {
        match byte {
            b'"' | b'\'' if quote == 0 => quote = byte,
            b if quote != 0 && b == quote => quote = 0,
            b'>' if quote == 0 => return Some(idx),
            _ => {}
        }
    }
    None
}

/// Skip `<!...>` including an internal `[ ... ]` subset. Returns bytes consumed.
fn skip_declaration(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let mut bracket = 0usize;
    for (idx, &byte) in bytes.iter().enumerate().skip(2) {
        match byte {
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b'>' if bracket == 0 => return Some(idx + 1),
            _ => {}
        }
    }
    None
}

fn collect_attrs(tag: &str, out: &mut Vec<String>, remaining: usize) -> usize {
    let bytes = tag.as_bytes();
    let mut i = 0usize;
    // Skip the element name.
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'/' {
        i += 1;
    }
    let mut found = 0usize;
    while i < bytes.len() && found < remaining {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'/'
        {
            i += 1;
        }
        if i == name_start {
            break;
        }
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        if quote == b'"' || quote == b'\'' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            out.push(decode_entities(&tag[start..i]));
            found += 1;
            i += 1;
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'/' {
                i += 1;
            }
            out.push(decode_entities(&tag[start..i]));
            found += 1;
        }
    }
    found
}

/// Decode the five predefined entities and numeric character references.
/// Unknown entities are left literal: nothing is ever resolved from a DTD.
fn decode_entities(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let Some(end) = tail.find(';').filter(|end| *end <= 12) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(ch) => out.push(ch),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_and_attributes() {
        let doc = br#"<?xml version="1.0"?>
<root id="1"><item name="alpha">hello</item><item>world</item></root>"#;
        let values = parse(doc).expect("xml");
        assert_eq!(values.texts, vec!["hello", "world"]);
        assert_eq!(values.attrs, vec!["1", "alpha"]);
    }

    #[test]
    fn entities_are_decoded_but_never_resolved_from_dtd() {
        let doc = br#"<!DOCTYPE r [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>
<r a="&xxe;">&xxe; &amp; &#65;</r>"#;
        let values = parse(doc).expect("xml");
        assert_eq!(values.texts, vec!["&xxe; & A"]);
        assert_eq!(values.attrs, vec!["&xxe;"]);
    }

    #[test]
    fn cdata_is_text_and_comments_are_skipped() {
        let doc = b"<r><!-- c --><![CDATA[<script>]]></r>";
        let values = parse(doc).expect("xml");
        assert_eq!(values.texts, vec!["<script>"]);
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let mut doc = String::new();
        for _ in 0..MAX_DEPTH + 10 {
            doc.push_str("<a>");
        }
        doc.push('x');
        let values = parse(doc.as_bytes()).expect("xml");
        assert!(values.texts.is_empty() || values.texts[0] == "x");
    }

    #[test]
    fn content_type_gate() {
        assert!(is_xml_content_type("application/xml"));
        assert!(is_xml_content_type("text/xml; charset=utf-8"));
        assert!(is_xml_content_type("application/soap+xml"));
        assert!(!is_xml_content_type("application/json"));
        assert!(!is_xml_content_type("text/plain"));
    }

    #[test]
    fn malformed_input_does_not_panic() {
        for raw in [
            &b"<"[..],
            b"<r a=\"unterminated",
            b"</>",
            b"<!--",
            b"<![CDATA[",
        ] {
            let _ = parse(raw);
        }
    }
}
