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
//! HMAC-SHA256 authenticated PoW challenge + grant token (WAF abuse seam).

use exyonq_waf_api::{ChallengeIssue, ChallengeVerifyResult, WafChallengeHandler};
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::{HashSet, VecDeque};
use std::net::IpAddr;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

const CHALLENGE_TTL_SECS: u64 = 60;
const GRANT_TTL_SECS: u64 = 300;
const DEFAULT_DIFFICULTY: u32 = 16;
const REPLAY_CAP: usize = 4096;
const COOKIE_NAME: &str = "exyonq_waf_grant";
const VERIFY_PATH: &str = "/.exyonq/waf-challenge";

/// Process-local challenge signing + PoW verification service.
pub struct ChallengeService {
    key: [u8; 32],
    difficulty: u32,
    replay: Mutex<ReplayCache>,
}

struct ReplayCache {
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl ReplayCache {
    fn new() -> Self {
        Self {
            order: VecDeque::new(),
            set: HashSet::new(),
        }
    }

    fn insert_if_new(&mut self, id: &str) -> bool {
        if self.set.contains(id) {
            return false;
        }
        if self.order.len() >= REPLAY_CAP {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        self.order.push_back(id.to_string());
        self.set.insert(id.to_string());
        true
    }
}

impl ChallengeService {
    /// Load key from `EXYONQ_WAF_CHALLENGE_KEY_FILE` (32 raw bytes) or OsRng.
    pub fn from_env_or_random() -> Result<Self, String> {
        let key = load_challenge_key()?;
        Ok(Self::with_key(key, DEFAULT_DIFFICULTY))
    }

    pub fn with_key(key: [u8; 32], difficulty: u32) -> Self {
        Self {
            key,
            difficulty: difficulty.clamp(1, 28),
            replay: Mutex::new(ReplayCache::new()),
        }
    }

    fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn hmac_hex(&self, msg: &[u8]) -> String {
        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(&self.key).expect("HMAC key length");
        mac.update(msg);
        hex_encode(&mac.finalize().into_bytes())
    }

    fn challenge_mac_msg(
        challenge_id: &str,
        client_ip: IpAddr,
        host: &str,
        expires: u64,
        difficulty: u32,
    ) -> String {
        format!(
            "chal|{}|{}|{}|{}|{}",
            challenge_id, client_ip, host, expires, difficulty
        )
    }

    fn grant_mac_msg(
        client_ip: IpAddr,
        host: &str,
        expires: u64,
        difficulty: u32,
        generation: u64,
    ) -> String {
        format!(
            "grant|{}|{}|{}|{}|{}",
            client_ip, host, expires, difficulty, generation
        )
    }

    fn issue_inner(
        &self,
        client_ip: IpAddr,
        host: Option<&str>,
        _path: &str,
        _generation: u64,
    ) -> ChallengeIssue {
        let host = host.unwrap_or("");
        let expires = Self::now_unix().saturating_add(CHALLENGE_TTL_SECS);
        let mut id_bytes = [0u8; 16];
        if getrandom::fill(&mut id_bytes).is_err() {
            return ChallengeIssue {
                status: 503,
                content_type: "text/plain; charset=utf-8",
                body: b"challenge unavailable\n".to_vec(),
            };
        }
        let challenge_id = hex_encode(&id_bytes);
        let mac = self.hmac_hex(
            Self::challenge_mac_msg(&challenge_id, client_ip, host, expires, self.difficulty)
                .as_bytes(),
        );
        let body = render_challenge_html(&challenge_id, &mac, expires, self.difficulty);
        ChallengeIssue {
            status: 403,
            content_type: "text/html; charset=utf-8",
            body: body.into_bytes(),
        }
    }

    fn verify_inner(
        &self,
        client_ip: IpAddr,
        host: Option<&str>,
        body: &[u8],
        generation: u64,
    ) -> ChallengeVerifyResult {
        let fail = || ChallengeVerifyResult::Fail {
            status: 403,
            content_type: "text/plain; charset=utf-8",
            body: b"challenge failed\n".to_vec(),
        };
        let form = parse_form_body(body);
        let challenge_id = form.get("challenge_id").map(String::as_str).unwrap_or("");
        let nonce = form.get("nonce").map(String::as_str).unwrap_or("");
        let mac = form.get("mac").map(String::as_str).unwrap_or("");
        let expires: u64 = form
            .get("expires")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let difficulty: u32 = form
            .get("difficulty")
            .and_then(|s| s.parse().ok())
            .unwrap_or(self.difficulty);
        let return_to = form.get("return_to").cloned();

        if challenge_id.is_empty() || nonce.is_empty() || mac.is_empty() || expires == 0 {
            return fail();
        }
        let now = Self::now_unix();
        if now > expires {
            return fail();
        }
        let host_s = host.unwrap_or("");
        let expect = self.hmac_hex(
            Self::challenge_mac_msg(challenge_id, client_ip, host_s, expires, difficulty)
                .as_bytes(),
        );
        if !ct_eq_hex(mac, &expect) {
            return fail();
        }
        if !pow_valid(challenge_id, nonce, difficulty) {
            return fail();
        }
        {
            let mut replay = self.replay.lock().expect("challenge replay lock");
            if !replay.insert_if_new(challenge_id) {
                return fail();
            }
        }

        let grant_exp = now.saturating_add(GRANT_TTL_SECS);
        let grant_mac = self.hmac_hex(
            Self::grant_mac_msg(client_ip, host_s, grant_exp, difficulty, generation).as_bytes(),
        );
        let token = format!("v1.{grant_exp}.{difficulty}.{generation}.{grant_mac}");
        let set_cookie = format!(
            "{COOKIE_NAME}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={GRANT_TTL_SECS}"
        );

        if let Some(loc) = return_to.filter(|s| s.starts_with('/') && !s.starts_with("//")) {
            ChallengeVerifyResult::Ok {
                status: 302,
                content_type: "text/plain; charset=utf-8",
                body: b"ok\n".to_vec(),
                set_cookie,
                location: Some(loc),
            }
        } else {
            ChallengeVerifyResult::Ok {
                status: 200,
                content_type: "application/json; charset=utf-8",
                body: br#"{"ok":true}"#.to_vec(),
                set_cookie,
                location: None,
            }
        }
    }

    fn grant_allows_inner(
        &self,
        client_ip: IpAddr,
        host: Option<&str>,
        cookie_header: Option<&[u8]>,
        _generation: u64,
    ) -> bool {
        let Some(raw) = cookie_header else {
            return false;
        };
        let Ok(cookie_str) = std::str::from_utf8(raw) else {
            return false;
        };
        let Some(token) = extract_cookie_value(cookie_str, COOKIE_NAME) else {
            return false;
        };
        // v1.<expires>.<difficulty>.<generation>.<mac>
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 5 || parts[0] != "v1" {
            return false;
        }
        let Ok(expires) = parts[1].parse::<u64>() else {
            return false;
        };
        let Ok(difficulty) = parts[2].parse::<u32>() else {
            return false;
        };
        let Ok(gen) = parts[3].parse::<u64>() else {
            return false;
        };
        let mac = parts[4];
        let now = Self::now_unix();
        if now > expires {
            return false;
        }
        let host_s = host.unwrap_or("");
        // Generation is embedded and MACed; prior generations remain valid until expiry
        // while the signing key is unchanged (key rotation invalidates all grants).
        let expect = self
            .hmac_hex(Self::grant_mac_msg(client_ip, host_s, expires, difficulty, gen).as_bytes());
        ct_eq_hex(mac, &expect)
    }
}

impl WafChallengeHandler for ChallengeService {
    fn issue(
        &self,
        client_ip: IpAddr,
        host: Option<&str>,
        path: &str,
        generation: u64,
    ) -> ChallengeIssue {
        self.issue_inner(client_ip, host, path, generation)
    }

    fn verify_post(
        &self,
        client_ip: IpAddr,
        host: Option<&str>,
        body: &[u8],
        generation: u64,
    ) -> ChallengeVerifyResult {
        self.verify_inner(client_ip, host, body, generation)
    }

    fn grant_allows(
        &self,
        client_ip: IpAddr,
        host: Option<&str>,
        cookie_header: Option<&[u8]>,
        generation: u64,
    ) -> bool {
        self.grant_allows_inner(client_ip, host, cookie_header, generation)
    }
}

fn load_challenge_key() -> Result<[u8; 32], String> {
    if let Ok(path) = std::env::var("EXYONQ_WAF_CHALLENGE_KEY_FILE") {
        if !path.is_empty() {
            return read_key_file(Path::new(&path));
        }
    }
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| format!("getrandom: {e}"))?;
    Ok(key)
}

fn read_key_file(path: &Path) -> Result<[u8; 32], String> {
    let bytes = std::fs::read(path).map_err(|e| format!("challenge key file: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "EXYONQ_WAF_CHALLENGE_KEY_FILE must be exactly 32 bytes (got {})",
            bytes.len()
        ));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

fn pow_valid(challenge_id: &str, nonce: &str, difficulty: u32) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(challenge_id.as_bytes());
    hasher.update(b":");
    hasher.update(nonce.as_bytes());
    let digest = hasher.finalize();
    leading_zero_bits(&digest) >= difficulty
}

fn leading_zero_bits(digest: &[u8]) -> u32 {
    let mut bits = 0u32;
    for byte in digest {
        if *byte == 0 {
            bits += 8;
            continue;
        }
        bits += byte.leading_zeros();
        break;
    }
    bits
}

fn render_challenge_html(challenge_id: &str, mac: &str, expires: u64, difficulty: u32) -> String {
    // Inline JS only — no CDN, no eval. PoW via SubtleCrypto SHA-256.
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="Cache-Control" content="no-store">
<title>Challenge</title>
</head>
<body>
<p>Verifying your browser…</p>
<script>
(async function () {{
  const challengeId = {cid};
  const mac = {mac};
  const expires = {expires};
  const difficulty = {difficulty};
  const verifyPath = {path};
  function hex(buf) {{
    return Array.from(new Uint8Array(buf)).map(b => b.toString(16).padStart(2, "0")).join("");
  }}
  async function sha256(str) {{
    const data = new TextEncoder().encode(str);
    return new Uint8Array(await crypto.subtle.digest("SHA-256", data));
  }}
  function leadingZeroBits(bytes) {{
    let bits = 0;
    for (let i = 0; i < bytes.length; i++) {{
      if (bytes[i] === 0) {{ bits += 8; continue; }}
      bits += Math.clz32(bytes[i]) - 24;
      break;
    }}
    return bits;
  }}
  let nonce = 0;
  for (;;) {{
    const h = await sha256(challengeId + ":" + String(nonce));
    if (leadingZeroBits(h) >= difficulty) break;
    nonce++;
    if ((nonce & 0xfff) === 0) await new Promise(r => setTimeout(r, 0));
  }}
  const body = new URLSearchParams({{
    challenge_id: challengeId,
    nonce: String(nonce),
    mac: mac,
    expires: String(expires),
    difficulty: String(difficulty),
    return_to: location.pathname + location.search
  }});
  const resp = await fetch(verifyPath, {{
    method: "POST",
    headers: {{ "content-type": "application/x-www-form-urlencoded" }},
    body: body.toString(),
    credentials: "same-origin",
    redirect: "follow"
  }});
  if (resp.redirected) {{ location.href = resp.url; return; }}
  if (resp.ok) {{ location.reload(); return; }}
  document.body.textContent = "challenge failed";
}})();
</script>
</body>
</html>
"##,
        cid = js_string(challenge_id),
        mac = js_string(mac),
        expires = expires,
        difficulty = difficulty,
        path = js_string(VERIFY_PATH),
    )
}

fn js_string(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn parse_form_body(body: &[u8]) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(text) = std::str::from_utf8(body) else {
        return map;
    };
    // Prefer form-urlencoded; also accept simple JSON {"challenge_id":...}.
    if text.trim_start().starts_with('{') {
        for key in [
            "challenge_id",
            "nonce",
            "mac",
            "expires",
            "difficulty",
            "return_to",
        ] {
            if let Some(v) = json_string_field(text, key) {
                map.insert(key.to_string(), v);
            }
        }
        return map;
    }
    for pair in text.split('&') {
        let mut it = pair.splitn(2, '=');
        let Some(k) = it.next() else { continue };
        let v = it.next().unwrap_or("");
        map.insert(url_decode(k), url_decode(v));
    }
    map
}

fn json_string_field(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let idx = text.find(&needle)?;
    let after = &text[idx + needle.len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = quoted.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                }
                '"' => return Some(out),
                other => out.push(other),
            }
        }
        None
    } else {
        let end = rest.find([',', '}']).unwrap_or(rest.len());
        Some(rest[..end].trim().to_string())
    }
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let h = hex_nibble(bytes[i + 1]);
                let l = hex_nibble(bytes[i + 2]);
                if let (Some(h), Some(l)) = (h, l) {
                    out.push((h << 4) | l);
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
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn extract_cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    for part in header.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(name) {
            if let Some(v) = rest.strip_prefix('=') {
                return Some(v);
            }
        }
    }
    None
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

fn ct_eq_hex(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn svc() -> ChallengeService {
        ChallengeService::with_key([7u8; 32], 8)
    }

    fn find_nonce(challenge_id: &str, difficulty: u32) -> String {
        let mut nonce = 0u64;
        loop {
            let n = nonce.to_string();
            if pow_valid(challenge_id, &n, difficulty) {
                return n;
            }
            nonce += 1;
        }
    }

    #[test]
    fn pow_verify_accepts_valid_nonce() {
        let id = "abcd";
        let n = find_nonce(id, 8);
        assert!(pow_valid(id, &n, 8));
        assert!(!pow_valid(id, "0", 20));
    }

    #[test]
    fn hmac_forge_fails() {
        let s = svc();
        let ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1));
        let issue = s.issue(ip, Some("example.com"), "/", 1);
        let html = String::from_utf8(issue.body).unwrap();
        assert!(html.contains("challengeId"));
        let body = b"challenge_id=deadbeef&nonce=1&mac=00&expires=9999999999&difficulty=8";
        match s.verify_post(ip, Some("example.com"), body, 1) {
            ChallengeVerifyResult::Fail { status, .. } => assert_eq!(status, 403),
            ChallengeVerifyResult::Ok { .. } => panic!("forged mac must fail"),
        }
    }

    #[test]
    fn expiry_rejects() {
        let s = svc();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let host = "h.test";
        let challenge_id = "aa";
        let expires = 1u64; // long past
        let mac = s.hmac_hex(
            ChallengeService::challenge_mac_msg(challenge_id, ip, host, expires, 8).as_bytes(),
        );
        let nonce = find_nonce(challenge_id, 8);
        let body = format!(
            "challenge_id={challenge_id}&nonce={nonce}&mac={mac}&expires={expires}&difficulty=8"
        );
        match s.verify_post(ip, Some(host), body.as_bytes(), 1) {
            ChallengeVerifyResult::Fail { .. } => {}
            ChallengeVerifyResult::Ok { .. } => panic!("expired must fail"),
        }
    }

    #[test]
    fn replay_rejected() {
        let s = svc();
        let ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 2));
        let host = "ex.test";
        let challenge_id = "bbccddee";
        let expires = ChallengeService::now_unix() + 60;
        let mac = s.hmac_hex(
            ChallengeService::challenge_mac_msg(challenge_id, ip, host, expires, 8).as_bytes(),
        );
        let nonce = find_nonce(challenge_id, 8);
        let body = format!(
            "challenge_id={challenge_id}&nonce={nonce}&mac={mac}&expires={expires}&difficulty=8"
        );
        match s.verify_post(ip, Some(host), body.as_bytes(), 3) {
            ChallengeVerifyResult::Ok { set_cookie, .. } => {
                assert!(set_cookie.contains(COOKIE_NAME));
            }
            ChallengeVerifyResult::Fail { .. } => panic!("first verify should pass"),
        }
        match s.verify_post(ip, Some(host), body.as_bytes(), 3) {
            ChallengeVerifyResult::Fail { .. } => {}
            ChallengeVerifyResult::Ok { .. } => panic!("replay must fail"),
        }
    }

    #[test]
    fn grant_survives_generation_bump() {
        let s = svc();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 9));
        let host = "g.test";
        let expires = ChallengeService::now_unix() + 300;
        let mac = s.hmac_hex(ChallengeService::grant_mac_msg(ip, host, expires, 8, 1).as_bytes());
        let token = format!("v1.{expires}.8.1.{mac}");
        let cookie = format!("{COOKIE_NAME}={token}");
        assert!(s.grant_allows(ip, Some(host), Some(cookie.as_bytes()), 2));
    }

    #[test]
    fn path_prefix_helper_not_here() {
        // unit: default difficulty constant
        assert_eq!(DEFAULT_DIFFICULTY, 16);
    }
}
