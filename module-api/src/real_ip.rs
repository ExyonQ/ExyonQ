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
//! NG-11: trust-aware client-IP resolution (nginx `set_real_ip_from` /
//! `real_ip_header`).
//!
//! - Disabled or empty `trusted` → the socket peer is the client IP; the
//!   client-supplied forwarding header is never consulted (spoof-proof).
//! - Peer inside `trusted` → walk the header chain right-to-left skipping
//!   trusted hops; the first untrusted entry is the real client. A malformed
//!   entry fails closed to the socket peer.
//! - Peer outside `trusted` → socket peer (an untrusted proxy cannot rewrite
//!   the client identity).

use exyonq_config_ir::RealIpConfig;
use ipnetwork::IpNetwork;
use std::net::IpAddr;

/// Parsed trusted-proxy configuration, built once per connection/request
/// entry instead of reparsing CIDR strings on every hop.
#[derive(Debug, Clone)]
pub struct RealIpTrust {
    /// Header carrying the forwarded chain (nginx `real_ip_header`).
    pub header: String,
    nets: Vec<IpNetwork>,
}

impl RealIpTrust {
    /// Direct construction from a header name + trusted CIDR/IP strings.
    /// Unparseable entries are skipped (config validation rejects them first).
    pub fn new(header: &str, trusted: &[String]) -> Self {
        let nets = trusted
            .iter()
            .filter_map(|t| {
                t.parse::<IpNetwork>()
                    .ok()
                    .or_else(|| t.trim().parse::<IpAddr>().ok().map(IpNetwork::from))
            })
            .collect();
        Self {
            header: header.trim().to_ascii_lowercase(),
            nets,
        }
    }

    /// `None` when the module is disabled or no trusted net is usable.
    pub fn from_config(cfg: &RealIpConfig) -> Option<Self> {
        if !cfg.enabled {
            return None;
        }
        let nets: Vec<IpNetwork> = cfg
            .trusted
            .iter()
            .filter_map(|t| {
                t.parse::<IpNetwork>()
                    .ok()
                    .or_else(|| t.trim().parse::<IpAddr>().ok().map(IpNetwork::from))
            })
            .collect();
        if nets.is_empty() {
            return None;
        }
        Some(Self {
            header: cfg.header.trim().to_ascii_lowercase(),
            nets,
        })
    }

    pub fn is_trusted(&self, ip: IpAddr) -> bool {
        // Canonical form folds ::ffff:a.b.c.d → a.b.c.d so an IPv4 trusted net
        // matches a v4-mapped peer on dual-stack binds (LA-CAP055 convention).
        let ip = ip.to_canonical();
        self.nets.iter().any(|n| n.contains(ip))
    }

    /// Resolve the effective client IP.
    ///
    /// `header_value` is the raw inbound value of `self.header`
    /// (comma-separated chain; leftmost = claimed original client,
    /// rightmost = last proxy hop).
    pub fn resolve(&self, peer: IpAddr, header_value: Option<&str>) -> IpAddr {
        if !self.is_trusted(peer) {
            return peer;
        }
        let Some(raw) = header_value else {
            return peer;
        };
        // Any malformed entry fails closed to the peer.
        let mut ips = Vec::new();
        for entry in raw.split(',').map(str::trim) {
            match entry.parse::<IpAddr>() {
                Ok(ip) => ips.push(ip),
                Err(_) => return peer,
            }
        }
        // nginx `real_ip_recursive` walk: right-to-left (nearest → farthest),
        // skipping trusted hops; the first untrusted entry is the client.
        // All-trusted chain (e.g. all-corporate hops) → leftmost wins.
        ips.iter()
            .rev()
            .find(|ip| !self.is_trusted(**ip))
            .or_else(|| ips.first())
            .copied()
            .unwrap_or(peer)
    }

    /// Look up `self.header` inside an already-parsed header list and resolve.
    /// Last occurrence wins (nginx semantics for repeated headers).
    pub fn resolve_from_pairs(&self, peer: IpAddr, pairs: &[(String, String)]) -> IpAddr {
        let value = pairs
            .iter()
            .rev()
            .find(|(n, _)| n.eq_ignore_ascii_case(&self.header))
            .map(|(_, v)| v.as_str());
        self.resolve(peer, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(enabled: bool, trusted: &[&str], header: &str) -> RealIpConfig {
        RealIpConfig {
            enabled,
            trusted: trusted.iter().map(|s| s.to_string()).collect(),
            header: header.to_string(),
        }
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn disabled_returns_peer_even_with_spoof_header() {
        assert!(
            RealIpTrust::from_config(&cfg(false, &["10.0.0.0/8"], "x-forwarded-for")).is_none()
        );
    }

    #[test]
    fn untrusted_peer_cannot_spoof() {
        let t = RealIpTrust::from_config(&cfg(true, &["10.0.0.0/8"], "x-forwarded-for")).unwrap();
        // Peer 192.0.2.1 is outside the trusted net → header ignored.
        assert_eq!(t.resolve(ip("192.0.2.1"), Some("6.6.6.6")), ip("192.0.2.1"));
    }

    #[test]
    fn one_trusted_proxy_recovers_client() {
        let t = RealIpTrust::from_config(&cfg(true, &["10.0.0.0/8"], "x-forwarded-for")).unwrap();
        assert_eq!(
            t.resolve(ip("10.0.0.5"), Some("203.0.113.7")),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn two_trusted_proxies_walk_right_to_left() {
        let t = RealIpTrust::from_config(&cfg(true, &["10.0.0.0/8"], "x-forwarded-for")).unwrap();
        // peer=10.0.0.9 (trusted LB), chain "client, 10.0.0.4" (trusted proxy)
        assert_eq!(
            t.resolve(ip("10.0.0.9"), Some("198.51.100.3, 10.0.0.4")),
            ip("198.51.100.3")
        );
    }

    #[test]
    fn untrusted_middle_hop_stops_walk() {
        let t = RealIpTrust::from_config(&cfg(true, &["10.0.0.0/8"], "x-forwarded-for")).unwrap();
        // client claimed 1.1.1.1 but entered via untrusted 192.0.2.50 — that
        // hop is the last verifiable identity.
        assert_eq!(
            t.resolve(ip("10.0.0.9"), Some("1.1.1.1, 192.0.2.50, 10.0.0.4")),
            ip("192.0.2.50")
        );
    }

    #[test]
    fn malformed_entry_fails_closed_to_peer() {
        let t = RealIpTrust::from_config(&cfg(true, &["10.0.0.0/8"], "x-forwarded-for")).unwrap();
        assert_eq!(
            t.resolve(ip("10.0.0.9"), Some("203.0.113.7, bogus")),
            ip("10.0.0.9")
        );
        // Port-suffixed junk is not an IP → fail closed.
        assert_eq!(
            t.resolve(ip("10.0.0.9"), Some("203.0.113.7:5678")),
            ip("10.0.0.9")
        );
    }

    #[test]
    fn missing_header_returns_peer() {
        let t = RealIpTrust::from_config(&cfg(true, &["10.0.0.0/8"], "x-forwarded-for")).unwrap();
        assert_eq!(t.resolve(ip("10.0.0.9"), None), ip("10.0.0.9"));
    }

    #[test]
    fn all_trusted_chain_returns_leftmost() {
        let t = RealIpTrust::from_config(&cfg(true, &["10.0.0.0/8"], "x-forwarded-for")).unwrap();
        assert_eq!(
            t.resolve(ip("10.0.0.9"), Some("10.0.0.1, 10.0.0.2")),
            ip("10.0.0.1")
        );
    }

    #[test]
    fn v4_mapped_v6_peer_matches_v4_trusted_net() {
        // Dual-stack binds deliver 10.0.0.5 as ::ffff:10.0.0.5 — the v4 CIDR
        // must still classify it as trusted.
        let t = RealIpTrust::from_config(&cfg(true, &["10.0.0.0/8"], "x-forwarded-for")).unwrap();
        let mapped: IpAddr = "::ffff:10.0.0.5".parse().unwrap();
        assert!(t.is_trusted(mapped));
        assert_eq!(t.resolve(mapped, Some("203.0.113.7")), ip("203.0.113.7"));
        // A mapped peer outside the v4 net stays untrusted.
        let outside: IpAddr = "::ffff:192.0.2.9".parse().unwrap();
        assert!(!t.is_trusted(outside));
        assert_eq!(
            t.resolve(outside, Some("203.0.113.7")),
            ip("::ffff:192.0.2.9")
        );
    }

    #[test]
    fn ipv6_and_single_ip_entries() {
        let t = RealIpTrust::from_config(&cfg(
            true,
            &["2001:db8::/32", "192.0.2.10"],
            "x-forwarded-for",
        ))
        .unwrap();
        assert_eq!(
            t.resolve(ip("2001:db8::1"), Some("203.0.113.9")),
            ip("203.0.113.9")
        );
        assert_eq!(
            t.resolve(ip("192.0.2.10"), Some("2001:db8:ffff::1")),
            ip("2001:db8:ffff::1")
        );
    }

    #[test]
    fn custom_header_and_last_occurrence_wins() {
        let t = RealIpTrust::from_config(&cfg(true, &["10.0.0.0/8"], "x-real-ip")).unwrap();
        let pairs = vec![
            ("x-real-ip".to_string(), "1.1.1.1".to_string()),
            ("x-real-ip".to_string(), "203.0.113.9".to_string()),
        ];
        assert_eq!(
            t.resolve_from_pairs(ip("10.0.0.9"), &pairs),
            ip("203.0.113.9")
        );
        // Case-insensitive header match.
        let pairs = vec![("X-Real-IP".to_string(), "203.0.113.8".to_string())];
        assert_eq!(
            t.resolve_from_pairs(ip("10.0.0.9"), &pairs),
            ip("203.0.113.8")
        );
    }

    #[test]
    fn empty_entries_are_malformed() {
        let t = RealIpTrust::from_config(&cfg(true, &["10.0.0.0/8"], "x-forwarded-for")).unwrap();
        assert_eq!(
            t.resolve(ip("10.0.0.9"), Some("203.0.113.7,,10.0.0.4")),
            ip("10.0.0.9")
        );
    }
}
