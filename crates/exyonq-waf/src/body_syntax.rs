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
//! Strict body-processor validation.
//!
//! ModSecurity answers `400` when its JSON or XML processor cannot parse the
//! request body (the same class of failure as its multipart parser). CRS
//! parity therefore needs the same rejection before any rule can score.

use crate::xml;

/// Returns the rejection reason when the body cannot be parsed by the
/// processor selected by `Content-Type`.
pub(crate) fn strict_error(content_type: &str, body: &[u8]) -> Option<&'static str> {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    if xml::is_xml_content_type(&mime) {
        return xml::strict_error(body);
    }

    if mime == "application/json" || mime.ends_with("+json") {
        // ModSecurity tolerates an absent/empty JSON body; it only rejects a
        // present body that is not valid JSON.
        if body.iter().all(u8::is_ascii_whitespace) {
            return None;
        }
        if serde_json::from_slice::<serde_json::Value>(body).is_err() {
            return Some("request body is not valid JSON");
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_json_is_rejected_but_empty_json_is_not() {
        assert!(strict_error("application/json", b"test").is_some());
        assert!(strict_error("application/json; charset=utf-8", b"{\"a\":").is_some());
        assert!(strict_error("application/vnd.api+json", b"[]").is_none());
        assert!(strict_error("application/json", b"").is_none());
        assert!(strict_error("application/json", b"   ").is_none());
    }

    #[test]
    fn xml_documents_must_be_well_formed() {
        assert!(strict_error("text/xml", b"").is_some());
        assert!(strict_error("application/xml", b"test").is_some());
        assert!(strict_error("application/soap+xml", b"<a><b></a>").is_some());
        assert!(strict_error("text/xml", b"<a></a><b></b>").is_some());
        assert!(strict_error("text/xml", b"<a>").is_some());
        assert!(strict_error("text/xml", b"<?xml version=\"1.0\"?><a x=\"1\">ok</a>").is_none());
        assert!(strict_error("application/xml", b"<a><b>ok</b></a>").is_none());
        assert!(strict_error(
            "text/xml",
            br#"<SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/"><SOAP-ENV:Body>ok</SOAP-ENV:Body></SOAP-ENV:Envelope>"#
        )
        .is_none());
    }

    #[test]
    fn backslash_before_attribute_quote_is_rejected() {
        let body = br#"<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"http://schemas.xmlsoap.org/soap/envelope/\"><SOAP-ENV:Body><xkms:RespondWith>%1Gwww.attack.org</xkms:RespondWith></SOAP-ENV:Body></SOAP-ENV:Envelope>"#;
        assert_eq!(
            strict_error("text/xml", body),
            Some("unquoted attribute value")
        );
    }
}
