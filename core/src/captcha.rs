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
//! OL-04 — server-side CAPTCHA provider verification.
//!
//! The secret is read from the environment variable named in config; it is
//! never serialized, logged or returned. Provider failures are fail-closed.

use bytes::Bytes;
use exyonq_config_ir::{WafAbuseConfig, WafCaptchaProvider};
use http_body_util::{BodyExt, Full};
use hyper::header::CONTENT_TYPE;
use hyper::Request;
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

const MAX_PROVIDER_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_PROVIDER_TOKEN_BYTES: usize = 4096;
const RECAPTCHA_V3_ACTION: &str = "challenge";
const RECAPTCHA_V3_MIN_SCORE: f64 = 0.5;

type ProviderClient = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;
static PROVIDER_CLIENT: OnceLock<Result<ProviderClient, String>> = OnceLock::new();

fn provider_client() -> Result<ProviderClient, String> {
    PROVIDER_CLIENT
        .get_or_init(|| {
            // Provider endpoints are HTTPS by default. The reverse-proxy
            // client is HTTP-only, so it cannot be reused here.
            let connector = HttpsConnectorBuilder::new()
                .with_native_roots()
                .map_err(|_| "provider TLS roots unavailable")?
                .https_or_http()
                .enable_http1()
                .build();
            Ok(Client::builder(TokioExecutor::new()).build(connector))
        })
        .clone()
}

/// Extract a form value from a bounded `application/x-www-form-urlencoded` body.
pub(crate) fn form_value(body: &[u8], name: &str) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?;
    for pair in text.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == name {
            return Some(url_decode(value));
        }
    }
    None
}

fn url_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    out.push(byte);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

fn provider_endpoint(abuse: &WafAbuseConfig) -> &str {
    abuse.verify_url.as_deref().unwrap_or(match abuse.provider {
        WafCaptchaProvider::Recaptcha | WafCaptchaProvider::RecaptchaV3 => {
            "https://www.google.com/recaptcha/api/siteverify"
        }
        WafCaptchaProvider::Hcaptcha => "https://hcaptcha.com/siteverify",
        WafCaptchaProvider::Native => "",
    })
}

/// Verify a provider token. `Ok(())` means the provider explicitly accepted it.
pub(crate) async fn verify_provider(
    abuse: &WafAbuseConfig,
    token: &str,
    remote_ip: IpAddr,
) -> Result<(), String> {
    if abuse.provider == WafCaptchaProvider::Native {
        return Ok(());
    }
    if token.is_empty() || token.len() > MAX_PROVIDER_TOKEN_BYTES {
        return Err("missing or oversized provider token".into());
    }
    let secret_env = abuse.secret_env.as_deref().unwrap_or("");
    if secret_env.is_empty() {
        return Err("provider secret environment is not configured".into());
    }
    let secret = std::env::var(secret_env).map_err(|_| "provider secret is unavailable")?;
    if secret.is_empty() {
        return Err("provider secret is empty".into());
    }
    let endpoint = provider_endpoint(abuse);
    let uri: hyper::Uri = endpoint
        .parse()
        .map_err(|_| "provider endpoint is invalid")?;
    if uri.scheme_str() != Some("https") && uri.scheme_str() != Some("http") {
        return Err("provider endpoint must be http(s)".into());
    }
    let form = format!(
        "secret={}&response={}&remoteip={}",
        percent_encode(&secret),
        percent_encode(token),
        percent_encode(&remote_ip.to_string())
    );
    let request = Request::builder()
        .method(hyper::Method::POST)
        .uri(uri)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body(Full::new(Bytes::from(form)))
        .map_err(|_| "provider request build failed")?;
    let client = provider_client()?;
    let response = tokio::time::timeout(Duration::from_secs(5), client.request(request))
        .await
        .map_err(|_| "provider verification timed out")?
        .map_err(|_| "provider verification failed")?;
    if response.status() != hyper::StatusCode::OK {
        return Err("provider returned a non-200 status".into());
    }
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| "provider response body failed")?;
        if let Some(data) = frame.data_ref() {
            if bytes.len().saturating_add(data.len()) > MAX_PROVIDER_RESPONSE_BYTES {
                return Err("provider response body is too large".into());
            }
            bytes.extend_from_slice(data);
        }
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "provider response is not JSON")?;
    validate_provider_response(abuse.provider, &value)
}

fn validate_provider_response(
    provider: WafCaptchaProvider,
    value: &serde_json::Value,
) -> Result<(), String> {
    if value.get("success").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err("provider rejected the token".into());
    }
    if provider != WafCaptchaProvider::RecaptchaV3 {
        return Ok(());
    }
    let action = value
        .get("action")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if action != RECAPTCHA_V3_ACTION {
        return Err("v3 action does not match the challenge action".into());
    }
    let Some(score) = value.get("score").and_then(serde_json::Value::as_f64) else {
        return Err("v3 score is missing".into());
    };
    if !score.is_finite() || !(0.0..=1.0).contains(&score) {
        return Err("v3 score is outside the valid range".into());
    }
    if score < RECAPTCHA_V3_MIN_SCORE {
        return Err("v3 score is below the minimum".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_value_decodes_provider_token() {
        assert_eq!(
            form_value(b"a=1&provider_token=abc%2Bdef", "provider_token"),
            Some("abc+def".into())
        );
        assert_eq!(form_value(b"a=1", "provider_token"), None);
    }

    #[test]
    fn percent_encoding_never_leaves_delimiters() {
        assert_eq!(percent_encode("a b&c=d"), "a%20b%26c%3Dd");
    }

    #[test]
    fn v3_requires_success_action_and_score() {
        let accepted = serde_json::json!({
            "success": true,
            "score": 0.7,
            "action": "challenge"
        });
        assert!(validate_provider_response(WafCaptchaProvider::RecaptchaV3, &accepted).is_ok());

        for rejected in [
            serde_json::json!({"success": true, "action": "challenge"}),
            serde_json::json!({"success": true, "score": 0.7}),
            serde_json::json!({"success": true, "score": 0.7, "action": "login"}),
            serde_json::json!({"success": true, "score": 0.3, "action": "challenge"}),
            serde_json::json!({"success": true, "score": 1.1, "action": "challenge"}),
            serde_json::json!({"success": false, "score": 0.9, "action": "challenge"}),
        ] {
            assert!(
                validate_provider_response(WafCaptchaProvider::RecaptchaV3, &rejected).is_err(),
                "unexpected acceptance: {rejected}"
            );
        }
    }

    #[test]
    fn v2_response_does_not_require_v3_fields() {
        let value = serde_json::json!({"success": true});
        assert!(validate_provider_response(WafCaptchaProvider::Recaptcha, &value).is_ok());
        assert!(validate_provider_response(WafCaptchaProvider::Hcaptcha, &value).is_ok());
    }
}
