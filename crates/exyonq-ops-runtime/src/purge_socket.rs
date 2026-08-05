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
//! WC3 — dedicated Unix socket for authenticated FPC purge (not lifecycle OpsCommand).

use exyonq_module_api::cache_purge::{
    CachePurgeOp, CachePurgeOutcome, CachePurgePort, CachePurgeSocketConfig,
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

const MAX_LINE_BYTES: usize = 4096;
const GLOBAL_RPS: u32 = 20;
const GLOBAL_BURST: u32 = 40;
const PER_SITE_RPS: u32 = 5;
const PER_SITE_BURST: u32 = 10;

/// Constant-time equality for auth tokens (length mismatch → false).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedPurge {
    Op { op: CachePurgeOp, token: Vec<u8> },
    Error(&'static str),
}

/// Parse one purge line.
///
/// ```text
/// purge site <site_id> <token>
/// purge url <site_id> <scheme> <host> <path> <token>
/// purge url <site_id> <scheme> <host> <path> <query> <token>
/// purge generation <site_id> <generation> <token>
/// purge tag <site_id> <tag> <token>
/// ```
pub fn parse_purge_line(line: &str) -> ParsedPurge {
    let line = line.trim();
    if line.is_empty() || line.len() > MAX_LINE_BYTES {
        return ParsedPurge::Error("invalid_key");
    }
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 || !parts[0].eq_ignore_ascii_case("purge") {
        return ParsedPurge::Error("unsupported_operation");
    }
    match parts[1].to_ascii_lowercase().as_str() {
        "site" => {
            if parts.len() != 4 {
                return ParsedPurge::Error("invalid_key");
            }
            let site_id = match parse_site_id(parts[2]) {
                Ok(id) => id,
                Err(e) => return ParsedPurge::Error(e),
            };
            ParsedPurge::Op {
                op: CachePurgeOp::Site { site_id },
                token: parts[3].as_bytes().to_vec(),
            }
        }
        "url" => {
            // 7 fields: … path token
            // 8 fields: … path query token
            if parts.len() != 7 && parts.len() != 8 {
                return ParsedPurge::Error("invalid_key");
            }
            let site_id = match parse_site_id(parts[2]) {
                Ok(id) => id,
                Err(e) => return ParsedPurge::Error(e),
            };
            let scheme = parts[3].to_ascii_lowercase();
            if scheme != "http" && scheme != "https" {
                return ParsedPurge::Error("invalid_key");
            }
            let host = parts[4];
            if host.is_empty() || host.contains('/') || host.contains('?') {
                return ParsedPurge::Error("invalid_key");
            }
            let path = parts[5];
            if !path.starts_with('/') || path.contains("..") || path.contains('?') {
                return ParsedPurge::Error("invalid_key");
            }
            let (query, token) = if parts.len() == 8 {
                (parts[6].to_string(), parts[7].as_bytes().to_vec())
            } else {
                (String::new(), parts[6].as_bytes().to_vec())
            };
            ParsedPurge::Op {
                op: CachePurgeOp::Url {
                    site_id,
                    scheme,
                    host: host.to_string(),
                    path: path.to_string(),
                    query,
                },
                token,
            }
        }
        "generation" => {
            if parts.len() != 5 {
                return ParsedPurge::Error("invalid_key");
            }
            let site_id = match parse_site_id(parts[2]) {
                Ok(id) => id,
                Err(e) => return ParsedPurge::Error(e),
            };
            let Ok(generation) = parts[3].parse::<u64>() else {
                return ParsedPurge::Error("invalid_key");
            };
            ParsedPurge::Op {
                op: CachePurgeOp::Generation {
                    site_id,
                    generation,
                },
                token: parts[4].as_bytes().to_vec(),
            }
        }
        "tag" => {
            if parts.len() != 5 {
                return ParsedPurge::Error("invalid_key");
            }
            let site_id = match parse_site_id(parts[2]) {
                Ok(id) => id,
                Err(e) => return ParsedPurge::Error(e),
            };
            ParsedPurge::Op {
                op: CachePurgeOp::Tag {
                    site_id,
                    tag: parts[3].to_string(),
                },
                token: parts[4].as_bytes().to_vec(),
            }
        }
        _ => ParsedPurge::Error("unsupported_operation"),
    }
}

fn parse_site_id(raw: &str) -> Result<u64, &'static str> {
    let id = raw.parse::<u64>().map_err(|_| "invalid_scope")?;
    if id == 0 {
        return Err("invalid_scope");
    }
    Ok(id)
}

struct TokenBucket {
    tokens: f64,
    last: Instant,
    rate: f64,
    burst: f64,
}

impl TokenBucket {
    fn new(rate: f64, burst: f64) -> Self {
        Self {
            tokens: burst,
            last: Instant::now(),
            rate,
            burst,
        }
    }

    fn try_take(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

struct PurgeRateLimiter {
    global: TokenBucket,
    per_site: HashMap<u64, TokenBucket>,
}

impl PurgeRateLimiter {
    fn new() -> Self {
        Self {
            global: TokenBucket::new(GLOBAL_RPS as f64, GLOBAL_BURST as f64),
            per_site: HashMap::new(),
        }
    }

    fn allow(&mut self, site_id: u64) -> bool {
        if !self.global.try_take() {
            return false;
        }
        let bucket = self
            .per_site
            .entry(site_id)
            .or_insert_with(|| TokenBucket::new(PER_SITE_RPS as f64, PER_SITE_BURST as f64));
        bucket.try_take()
    }
}

#[derive(Serialize)]
struct PurgeResponseJson {
    ok: bool,
    command: &'static str,
    site_id: u64,
    purged_entries: u64,
    purged_bytes: u64,
    generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
}

fn outcome_json(outcome: &CachePurgeOutcome) -> PurgeResponseJson {
    PurgeResponseJson {
        ok: outcome.ok,
        command: outcome.operation,
        site_id: outcome.site_id,
        purged_entries: outcome.purged_entries,
        purged_bytes: outcome.purged_bytes,
        generation: outcome.generation,
        error: outcome.error,
    }
}

fn site_id_of(op: &CachePurgeOp) -> u64 {
    match op {
        CachePurgeOp::Url { site_id, .. }
        | CachePurgeOp::Site { site_id }
        | CachePurgeOp::Generation { site_id, .. }
        | CachePurgeOp::Tag { site_id, .. } => *site_id,
    }
}

fn op_label(op: &CachePurgeOp) -> &'static str {
    match op {
        CachePurgeOp::Url { .. } => "purge.url",
        CachePurgeOp::Site { .. } => "purge.site",
        CachePurgeOp::Generation { .. } => "purge.generation",
        CachePurgeOp::Tag { .. } => "purge.tag",
    }
}

async fn handle_client(
    stream: UnixStream,
    port: Arc<dyn CachePurgePort>,
    expected_token: Arc<[u8]>,
    limiter: Arc<Mutex<PurgeRateLimiter>>,
) -> anyhow::Result<()> {
    // Bound the read before materializing the line (security-compiler F1).
    let take = stream.take((MAX_LINE_BYTES as u64) + 1);
    let mut reader = BufReader::new(take);
    let mut command = String::new();
    reader.read_line(&mut command).await?;
    let mut stream = reader.into_inner().into_inner();

    if command.len() > MAX_LINE_BYTES {
        let payload = serde_json::to_string(&PurgeResponseJson {
            ok: false,
            command: "purge",
            site_id: 0,
            purged_entries: 0,
            purged_bytes: 0,
            generation: 0,
            error: Some("invalid_key"),
        })? + "\n";
        stream.write_all(payload.as_bytes()).await?;
        return Ok(());
    }

    let outcome = match parse_purge_line(&command) {
        ParsedPurge::Error(err) => {
            exyonq_cache::note_fpc_purge_request();
            exyonq_cache::note_fpc_purge_rejected(err);
            CachePurgeOutcome::fail("purge", 0, 0, err)
        }
        ParsedPurge::Op { op, token } => {
            if expected_token.is_empty() || !constant_time_eq(&token, expected_token.as_ref()) {
                exyonq_cache::note_fpc_purge_request();
                exyonq_cache::note_fpc_purge_rejected("unauthenticated");
                CachePurgeOutcome::fail(op_label(&op), site_id_of(&op), 0, "unauthenticated")
            } else {
                let site = site_id_of(&op);
                let allowed = limiter.lock().map(|mut g| g.allow(site)).unwrap_or(false);
                if !allowed {
                    exyonq_cache::note_fpc_purge_request();
                    exyonq_cache::note_fpc_purge_rejected("rate_limited");
                    CachePurgeOutcome::fail(op_label(&op), site, 0, "rate_limited")
                } else {
                    port.purge(op)
                }
            }
        }
    };

    let payload = serde_json::to_string(&outcome_json(&outcome))? + "\n";
    stream.write_all(payload.as_bytes()).await?;
    Ok(())
}

fn chmod_owner_only(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

pub async fn run_cache_purge_socket(
    config: CachePurgeSocketConfig,
    port: Arc<dyn CachePurgePort>,
) -> anyhow::Result<()> {
    let socket_path = &config.socket_path;
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }
    if let Some(parent) = socket_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let listener = UnixListener::bind(socket_path)?;
    chmod_owner_only(socket_path);
    info!(path = %socket_path.display(), "cache purge socket listening");

    let limiter = Arc::new(Mutex::new(PurgeRateLimiter::new()));
    let token = config.token;

    loop {
        let (stream, _) = listener.accept().await?;
        let port = Arc::clone(&port);
        let token = Arc::clone(&token);
        let limiter = Arc::clone(&limiter);
        tokio::spawn(async move {
            if let Err(err) = handle_client(stream, port, token, limiter).await {
                warn!(%err, "cache purge client error");
            }
        });
    }
}

pub fn spawn_cache_purge_socket(config: CachePurgeSocketConfig, port: Arc<dyn CachePurgePort>) {
    tokio::spawn(async move {
        if let Err(err) = run_cache_purge_socket(config.clone(), port).await {
            warn!(
                path = %config.socket_path.display(),
                %err,
                "cache purge socket stopped"
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_site_and_url() {
        match parse_purge_line("purge site 42 secret") {
            ParsedPurge::Op {
                op: CachePurgeOp::Site { site_id: 42 },
                token,
            } => assert_eq!(token, b"secret"),
            other => panic!("unexpected {other:?}"),
        }
        match parse_purge_line("purge url 42 http example.test /public/index.php secret") {
            ParsedPurge::Op {
                op:
                    CachePurgeOp::Url {
                        site_id: 42,
                        scheme,
                        host,
                        path,
                        query,
                    },
                ..
            } => {
                assert_eq!(scheme, "http");
                assert_eq!(host, "example.test");
                assert_eq!(path, "/public/index.php");
                assert!(query.is_empty());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn reject_path_traversal() {
        assert!(matches!(
            parse_purge_line("purge url 1 http h /../etc/passwd t"),
            ParsedPurge::Error("invalid_key")
        ));
    }

    #[test]
    fn constant_time_eq_basic() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn auth_rejects_wrong_and_empty_token() {
        let expected: Arc<[u8]> = Arc::from(b"secret".as_slice());
        assert!(!constant_time_eq(b"wrong", expected.as_ref()));
        assert!(!constant_time_eq(b"", expected.as_ref()));
        let empty: Arc<[u8]> = Arc::from(b"".as_slice());
        // Empty expected token must never authenticate (fail-closed).
        assert!(empty.is_empty());
        assert!(!constant_time_eq(b"secret", empty.as_ref()));
    }

    #[test]
    fn rate_limiter_eventually_trips() {
        let mut lim = PurgeRateLimiter::new();
        let mut denied = 0usize;
        for _ in 0..200 {
            if !lim.allow(7) {
                denied += 1;
            }
        }
        assert!(denied > 0, "per-site/global bucket must deny under flood");
    }
}
