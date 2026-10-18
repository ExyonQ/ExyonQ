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
//! Q-02 / ADR-054 — failure-only `EXY-RT-*` serve-path diagnostics.

use crate::retry::UpstreamAttemptClass;
use tracing::warn;

pub const EXY_RT_0001: &str = "EXY-RT-0001";
pub const EXY_RT_0002: &str = "EXY-RT-0002";
pub const EXY_RT_0003: &str = "EXY-RT-0003";
pub const EXY_RT_0004: &str = "EXY-RT-0004";
pub const EXY_RT_0005: &str = "EXY-RT-0005";

/// Borrowed context for a final proxy failure (after Cap021 retries exhausted).
#[derive(Debug, Clone, Copy)]
pub struct RtFailCtx<'a> {
    pub cluster_id: u32,
    pub path: &'a str,
    pub peer: Option<&'a str>,
    pub host: Option<&'a str>,
    /// Optional transport error text (DNS vs connect classification).
    pub detail: Option<&'a str>,
}

/// True when a connect-class error message looks like DNS resolution failure.
pub fn looks_like_dns(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.contains("dns")
        || lower.contains("failed to lookup")
        || lower.contains("name or service not known")
        || lower.contains("nodename nor servname")
        || lower.contains("no such host")
        || lower.contains("name resolution")
}

pub fn cause_for_connect(detail: Option<&str>) -> &'static str {
    match detail {
        Some(d) if looks_like_dns(d) => "dns",
        _ => "connect",
    }
}

pub fn code_for_class(class: UpstreamAttemptClass) -> &'static str {
    match class {
        UpstreamAttemptClass::ConnectFailed => EXY_RT_0001,
        UpstreamAttemptClass::TimedOut => EXY_RT_0002,
        UpstreamAttemptClass::UnretryableError => EXY_RT_0004,
    }
}

/// Emit a final upstream-class failure diagnostic (once per failed request).
pub fn emit_upstream_class(ctx: &RtFailCtx<'_>, class: UpstreamAttemptClass) {
    let code = code_for_class(class);
    let peer = ctx.peer.unwrap_or("");
    let host = ctx.host.unwrap_or("");
    match class {
        UpstreamAttemptClass::ConnectFailed => {
            let cause = cause_for_connect(ctx.detail);
            warn!(
                code,
                cause,
                cluster_id = ctx.cluster_id,
                peer,
                host,
                path = ctx.path,
                detail = ctx.detail.unwrap_or(""),
                "EXY-RT-0001: upstream connect failed"
            );
        }
        UpstreamAttemptClass::TimedOut => {
            warn!(
                code,
                cause = "timeout",
                cluster_id = ctx.cluster_id,
                peer,
                host,
                path = ctx.path,
                "EXY-RT-0002: upstream attempt timeout"
            );
        }
        UpstreamAttemptClass::UnretryableError => {
            warn!(
                code,
                cause = "upstream",
                cluster_id = ctx.cluster_id,
                peer,
                host,
                path = ctx.path,
                detail = ctx.detail.unwrap_or(""),
                "EXY-RT-0004: upstream unretryable error"
            );
        }
    }
}

/// No eligible peer / missing cluster binding → 503.
pub fn emit_no_eligible(ctx: &RtFailCtx<'_>) {
    warn!(
        code = EXY_RT_0003,
        cause = "no_peer",
        cluster_id = ctx.cluster_id,
        peer = ctx.peer.unwrap_or(""),
        host = ctx.host.unwrap_or(""),
        path = ctx.path,
        "EXY-RT-0003: no eligible upstream peer"
    );
}

/// `$map` name referenced but not defined.
pub fn emit_map_unknown(map_name: &str, path: &str, host: Option<&str>) {
    warn!(
        code = EXY_RT_0005,
        cause = "map_unknown",
        map = map_name,
        path,
        host = host.unwrap_or(""),
        "EXY-RT-0005: proxy_set_headers map variable unknown"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_heuristics() {
        assert!(looks_like_dns("dns error: failed to lookup address information"));
        assert!(looks_like_dns("Name or service not known"));
        assert!(!looks_like_dns("Connection refused (os error 61)"));
        assert_eq!(cause_for_connect(Some("dns error")), "dns");
        assert_eq!(cause_for_connect(Some("connection refused")), "connect");
    }

    #[test]
    fn codes_match_class() {
        assert_eq!(
            code_for_class(UpstreamAttemptClass::ConnectFailed),
            EXY_RT_0001
        );
        assert_eq!(code_for_class(UpstreamAttemptClass::TimedOut), EXY_RT_0002);
        assert_eq!(
            code_for_class(UpstreamAttemptClass::UnretryableError),
            EXY_RT_0004
        );
    }
}
