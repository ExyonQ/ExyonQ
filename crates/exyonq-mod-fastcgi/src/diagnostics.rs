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
//! Q-08 / ADR-060 — actionable FastCGI pool saturation diagnostics.

use crate::fcgi_stream::PoolEndpoint;

/// Timeout / busy class reported in the saturation warn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaturationKind {
    Checkout,
    Drain,
    Capacity,
    Closed,
}

impl SaturationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SaturationKind::Checkout => "checkout",
            SaturationKind::Drain => "drain",
            SaturationKind::Capacity => "capacity",
            SaturationKind::Closed => "closed",
        }
    }

    pub fn recommendation(self) -> &'static str {
        match self {
            SaturationKind::Checkout => {
                "raise [[fcgi_pool]].max_concurrency / checkout_timeout_ms, or speed up PHP-FPM workers"
            }
            SaturationKind::Drain => "wait for reload drain to finish; do not send new FastCGI work",
            SaturationKind::Capacity => {
                "raise max_connections (≤ max_concurrency) or reduce concurrent FastCGI load"
            }
            SaturationKind::Closed => "pool is closed/draining; retry after reload completes",
        }
    }
}

/// Format endpoint for logs (no secrets).
pub fn endpoint_display(endpoint: &PoolEndpoint) -> String {
    match endpoint {
        PoolEndpoint::Unix(p) => format!("unix:{}", p.display()),
        PoolEndpoint::Tcp(addr) => format!("tcp:{addr}"),
    }
}

/// Human one-liner used by ConnPool busy paths (also unit-tested).
pub fn format_saturation_message(
    pool: &str,
    kind: SaturationKind,
    open: usize,
    idle: usize,
    waiters: usize,
    max_connections: usize,
    endpoint: &str,
) -> String {
    format!(
        "fcgi pool saturated: pool={pool} timeout_kind={} open_sockets={open} idle_sockets={idle} waiters={waiters} max_connections={max_connections} endpoint={endpoint} recommendation={}",
        kind.as_str(),
        kind.recommendation()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_names_timeout_and_recommendation() {
        let msg = format_saturation_message(
            "php",
            SaturationKind::Checkout,
            4,
            0,
            2,
            4,
            "unix:/run/php.sock",
        );
        assert!(msg.contains("timeout_kind=checkout"));
        assert!(msg.contains("pool=php"));
        assert!(msg.contains("open_sockets=4"));
        assert!(msg.contains("recommendation="));
        assert!(msg.contains("max_concurrency"));
    }
}
