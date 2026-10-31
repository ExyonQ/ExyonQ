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
//! 0.5 — nginx `auth_basic` with htpasswd (bcrypt, {SHA}, {PLAIN}, plain).

use base64::Engine as _;
use exyonq_config_ir::AuthConfig;
use sha1::{Digest, Sha1};

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn verify(hash: &str, password: &str) -> bool {
    if hash.starts_with("$2a$") || hash.starts_with("$2b$") || hash.starts_with("$2y$") {
        return bcrypt::verify(password, hash).unwrap_or(false);
    }
    if let Some(rest) = hash.strip_prefix("{SHA}") {
        let digest = Sha1::digest(password.as_bytes());
        return base64::engine::general_purpose::STANDARD.encode(digest) == rest;
    }
    if let Some(rest) = hash.strip_prefix("{PLAIN}") {
        return rest == password;
    }
    // apr1/other crypt formats are not supported: fail closed.
    if hash.starts_with('$') {
        return false;
    }
    hash == password
}

/// Returns the realm when the request must be challenged.
pub(crate) fn check(
    config: &AuthConfig,
    route_name: &str,
    headers: &[(String, String)],
) -> Option<String> {
    let route = config.routes.get(route_name)?;
    let Some(value) = header_value(headers, "authorization") else {
        return Some(route.realm.clone());
    };
    let Some(encoded) = value
        .strip_prefix("Basic ")
        .or_else(|| value.strip_prefix("basic "))
    else {
        return Some(route.realm.clone());
    };
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok());
    let Some((user, password)) = decoded.as_deref().and_then(|v| v.split_once(':')) else {
        return Some(route.realm.clone());
    };
    let Ok(contents) = std::fs::read_to_string(&route.user_file) else {
        return Some(route.realm.clone());
    };
    for line in contents.lines() {
        let Some((name, hash)) = line.split_once(':') else {
            continue;
        };
        if name == user && verify(hash, password) {
            return None;
        }
    }
    Some(route.realm.clone())
}
