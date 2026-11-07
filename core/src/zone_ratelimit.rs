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
//! 0.5 — nginx `limit_req_zone` per-route leaky buckets.
//!
//! Key sources: `ip`, `host`, `route`, `user`, `header:<name>`,
//! `cookie:<name>`, and `expr:<nginx template>` for combined variables
//! (`expr:$remote_addr:$uri`). The state follows nginx's `excess` model: the
//! first request for a key is free, subsequent requests add one unit of excess
//! and time drains it at `requests_per_second`. `burst` is the maximum accepted
//! excess; `nodelay` accepts excess immediately, while `delay=N` delays only
//! the excess above N. An empty key is not accounted, matching nginx.

use exyonq_config_ir::{RateLimitConfig, RateLimitRouteOptions, RateLimitRoutePolicy};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

struct Bucket {
    excess: f64,
    last: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyContext<'a> {
    pub route_name: &'a str,
    pub headers: &'a [(String, String)],
    pub host: Option<&'a str>,
    pub client_ip: &'a str,
    pub user: Option<&'a str>,
    pub method: &'a str,
    pub path: &'a str,
    pub request_uri: &'a str,
    pub query: &'a str,
    pub scheme: &'a str,
    pub server_port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LimitDecision {
    Allow,
    Delay {
        wait: Duration,
        log_level: &'static str,
    },
    Reject {
        status: u16,
        retry_after_secs: u64,
        log_level: &'static str,
    },
    /// nginx `limit_req_dry_run on`: the request is accounted and logged, but
    /// neither delayed nor rejected.
    DryRun {
        status: u16,
        retry_after_secs: u64,
        log_level: &'static str,
    },
}

static ZONES: LazyLock<Mutex<HashMap<String, HashMap<String, Bucket>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// A fully-drained bucket is equivalent to an absent one. The LRU fallback
// bounds pathological key floods even when every bucket is active.
const MAX_KEYS_PER_ZONE: usize = 10_000;
const MAX_KEY_CHARS: usize = 128;

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn cookie_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("cookie"))
        .flat_map(|(_, value)| value.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .find(|(cookie_name, _)| *cookie_name == name)
        .map(|(_, value)| value.trim())
}

fn query_arg<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        (key == name).then_some(value)
    })
}

fn variable_value(ctx: &KeyContext<'_>, name: &str) -> String {
    match name {
        "binary_remote_addr" | "remote_addr" => ctx.client_ip.to_string(),
        "host" | "server_name" => ctx.host.unwrap_or_default().to_string(),
        "remote_user" => ctx.user.unwrap_or_default().to_string(),
        "uri" | "document_uri" => ctx.path.to_string(),
        "request_uri" => ctx.request_uri.to_string(),
        "args" => ctx.query.to_string(),
        "is_args" => {
            if ctx.query.is_empty() {
                String::new()
            } else {
                "?".to_string()
            }
        }
        "request_method" => ctx.method.to_string(),
        "scheme" => ctx.scheme.to_string(),
        "server_port" => ctx.server_port.to_string(),
        _ if name.starts_with("http_") => {
            let header = name["http_".len()..].replace('_', "-");
            header_value(ctx.headers, &header)
                .unwrap_or_default()
                .to_string()
        }
        _ if name.starts_with("cookie_") => cookie_value(ctx.headers, &name["cookie_".len()..])
            .unwrap_or_default()
            .to_string(),
        _ if name.starts_with("arg_") => query_arg(ctx.query, &name["arg_".len()..])
            .unwrap_or_default()
            .to_string(),
        // nginx evaluates an unknown variable as an empty string.
        _ => String::new(),
    }
}

/// Expand an nginx expression. Literal text is preserved; `$name` and
/// `${name}` are replaced by the request value. Unknown variables are empty,
/// which is nginx's evaluation behaviour for a variable it does not know.
fn expand_expression(expression: &str, ctx: &KeyContext<'_>) -> String {
    let mut out = String::with_capacity(expression.len());
    let mut chars = expression.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }
        let braced = matches!(chars.peek(), Some((_, '{')));
        if braced {
            chars.next();
        }
        let mut name = String::new();
        while let Some((_, next)) = chars.peek().copied() {
            if next.is_ascii_alphanumeric() || next == '_' {
                name.push(next);
                chars.next();
            } else {
                break;
            }
        }
        if name.is_empty() {
            // A lone `$` is literal text in nginx.
            out.push('$');
            continue;
        }
        out.push_str(&variable_value(ctx, &name));
        if braced && matches!(chars.peek(), Some((_, '}'))) {
            chars.next();
        }
    }
    out
}

fn legacy_key_value(key: &str, ctx: &KeyContext<'_>) -> Option<String> {
    match key {
        "ip" => Some(ctx.client_ip.to_string()),
        "host" => Some(ctx.host.unwrap_or_default().to_string()),
        "route" => Some(ctx.route_name.to_string()),
        "user" => Some(ctx.user.unwrap_or_default().to_string()),
        key if key.starts_with("header:") => Some(
            header_value(ctx.headers, &key["header:".len()..])
                .unwrap_or_default()
                .to_string(),
        ),
        key if key.starts_with("cookie:") => Some(
            cookie_value(ctx.headers, &key["cookie:".len()..])
                .unwrap_or_default()
                .to_string(),
        ),
        key if key.starts_with("expr:") => Some(expand_expression(&key["expr:".len()..], ctx)),
        _ => None,
    }
}

fn projected_excess(bucket: &Bucket, now: Instant, rps: f64) -> f64 {
    let elapsed = now
        .checked_duration_since(bucket.last)
        .unwrap_or(Duration::ZERO)
        .as_secs_f64();
    (bucket.excess - elapsed * rps).max(0.0)
}

/// Returns the action for this request. Rejections do not mutate the bucket,
/// matching nginx: a refused request must not consume a future slot.
pub(crate) fn check(config: &RateLimitConfig, ctx: &KeyContext<'_>) -> LimitDecision {
    check_at(config, ctx, Instant::now())
}

fn check_at(config: &RateLimitConfig, ctx: &KeyContext<'_>, now: Instant) -> LimitDecision {
    if let Some(policy) = config.route_limits.get(ctx.route_name) {
        return check_policy(config, policy, ctx, now);
    }
    let Some(zone_name) = config.routes.get(ctx.route_name) else {
        return LimitDecision::Allow;
    };
    let Some(zone) = config.zones.get(zone_name) else {
        return LimitDecision::Allow;
    };
    let options = config.route_options.get(ctx.route_name);
    // Configs written before route_options existed kept the legacy
    // reject-on-empty (nodelay) behavior. Imported nginx configs always carry
    // route_options/route_limits and therefore get nginx's delay semantics.
    let options = options.cloned().unwrap_or(RateLimitRouteOptions {
        burst: None,
        delay: None,
        nodelay: true,
    });
    let burst = options.burst.unwrap_or(zone.burst);
    let delay_limit = if options.nodelay {
        f64::INFINITY
    } else {
        options.delay.unwrap_or(0) as f64
    };
    let Some(key) = legacy_key_value(&zone.key, ctx) else {
        return LimitDecision::Allow;
    };
    match check_key(
        zone_name,
        zone.requests_per_second.max(1),
        burst as f64,
        delay_limit,
        key,
        now,
    ) {
        RawDecision::Allow => LimitDecision::Allow,
        RawDecision::Delay(wait) => LimitDecision::Delay {
            wait,
            log_level: "error",
        },
        RawDecision::Reject { retry_after_secs } => LimitDecision::Reject {
            status: 429,
            retry_after_secs,
            log_level: "error",
        },
    }
}

fn check_policy(
    config: &RateLimitConfig,
    policy: &RateLimitRoutePolicy,
    ctx: &KeyContext<'_>,
    now: Instant,
) -> LimitDecision {
    let log_level = match policy.log_level.as_str() {
        "info" => "info",
        "notice" => "notice",
        "warn" => "warn",
        _ => "error",
    };
    let mut delay = Duration::ZERO;
    let mut rejected = None;
    let mut dry_run_hit = false;
    for limit in &policy.limits {
        let Some(zone) = config.zones.get(&limit.zone) else {
            continue;
        };
        let burst = limit.burst.unwrap_or(zone.burst);
        let delay_limit = if limit.nodelay {
            f64::INFINITY
        } else {
            limit.delay.unwrap_or(0) as f64
        };
        let Some(key) = legacy_key_value(&zone.key, ctx) else {
            continue;
        };
        match check_key(
            &limit.zone,
            zone.requests_per_second.max(1),
            burst as f64,
            delay_limit,
            key,
            now,
        ) {
            RawDecision::Allow => {}
            RawDecision::Delay(wait) => {
                if policy.dry_run {
                    dry_run_hit = true;
                }
                delay = delay.max(wait);
            }
            RawDecision::Reject { retry_after_secs } => {
                dry_run_hit = true;
                rejected = Some(retry_after_secs);
            }
        }
    }
    if policy.dry_run {
        if let Some(retry_after_secs) = rejected {
            return LimitDecision::DryRun {
                status: policy.status,
                retry_after_secs,
                log_level,
            };
        }
        if dry_run_hit {
            return LimitDecision::DryRun {
                status: policy.status,
                retry_after_secs: 1,
                log_level,
            };
        }
        return LimitDecision::Allow;
    }
    if let Some(retry_after_secs) = rejected {
        return LimitDecision::Reject {
            status: policy.status,
            retry_after_secs,
            log_level,
        };
    }
    if delay > Duration::ZERO {
        return LimitDecision::Delay {
            wait: delay,
            log_level,
        };
    }
    LimitDecision::Allow
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawDecision {
    Allow,
    Delay(Duration),
    Reject { retry_after_secs: u64 },
}

fn check_key(
    zone_name: &str,
    rps: u32,
    burst: f64,
    delay_limit: f64,
    key: String,
    now: Instant,
) -> RawDecision {
    if key.is_empty() {
        // nginx does not account requests with an empty key.
        return RawDecision::Allow;
    }
    let key: String = key.chars().take(MAX_KEY_CHARS).collect();
    let rps = rps as f64;

    let mut zones = ZONES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let map = zones.entry(zone_name.to_string()).or_default();
    if map.len() >= MAX_KEYS_PER_ZONE && !map.contains_key(&key) {
        map.retain(|_, bucket| projected_excess(bucket, now, rps) > 0.0);
        if map.len() >= MAX_KEYS_PER_ZONE {
            if let Some(oldest) = map
                .iter()
                .max_by_key(|(_, bucket)| bucket.last.elapsed())
                .map(|(key, _)| key.clone())
            {
                map.remove(&oldest);
            }
        }
    }

    let bucket = match map.entry(key) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            // nginx's first request for a key starts at excess 0 and is free.
            entry.insert(Bucket {
                excess: 0.0,
                last: now,
            });
            return RawDecision::Allow;
        }
        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
    };

    let elapsed = now
        .checked_duration_since(bucket.last)
        .unwrap_or(Duration::ZERO)
        .as_secs_f64();
    let excess = (bucket.excess - elapsed * rps + 1.0).max(0.0);
    if excess > burst {
        let retry = ((excess - burst) / rps).ceil().max(1.0) as u64;
        return RawDecision::Reject {
            retry_after_secs: retry,
        };
    }
    bucket.excess = excess;
    bucket.last = now;

    if excess > delay_limit {
        let wait_seconds = ((excess - delay_limit) / rps).max(0.0);
        if wait_seconds > 0.0 {
            return RawDecision::Delay(Duration::from_secs_f64(wait_seconds));
        }
    }
    RawDecision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_config_ir::{
        RateLimitLimit, RateLimitRouteOptions, RateLimitRoutePolicy, RateLimitZoneConfig,
    };

    fn config(zone_name: &str, key: &str, route_names: &[&str]) -> RateLimitConfig {
        let mut cfg = RateLimitConfig::default();
        cfg.zones.insert(
            zone_name.to_string(),
            RateLimitZoneConfig {
                key: key.to_string(),
                requests_per_second: 1,
                burst: 1,
            },
        );
        for route in route_names {
            cfg.routes
                .insert((*route).to_string(), zone_name.to_string());
        }
        cfg
    }

    fn with_options(
        mut cfg: RateLimitConfig,
        route: &str,
        burst: Option<u32>,
        delay: Option<u32>,
        nodelay: bool,
    ) -> RateLimitConfig {
        cfg.route_options.insert(
            route.to_string(),
            RateLimitRouteOptions {
                burst,
                delay,
                nodelay,
            },
        );
        cfg
    }

    fn ctx<'a>(route: &'a str, headers: &'a [(String, String)]) -> KeyContext<'a> {
        KeyContext {
            route_name: route,
            headers,
            host: Some("example.test"),
            client_ip: "127.0.0.1",
            user: None,
            method: "GET",
            path: "/resource",
            request_uri: "/resource?x=1",
            query: "x=1",
            scheme: "http",
            server_port: 80,
        }
    }

    #[test]
    fn route_key_isolates_buckets_per_route() {
        let cfg = config("ng19-route-zone", "route", &["r1", "r2"]);
        assert_eq!(check(&cfg, &ctx("r1", &[])), LimitDecision::Allow);
        // burst=1 permits the first excess request; the next one overflows.
        assert_eq!(check(&cfg, &ctx("r1", &[])), LimitDecision::Allow);
        assert!(matches!(
            check(&cfg, &ctx("r1", &[])),
            LimitDecision::Reject { .. }
        ));
        assert_eq!(check(&cfg, &ctx("r2", &[])), LimitDecision::Allow);
    }

    #[test]
    fn cookie_key_reads_the_named_cookie() {
        let cfg = config("ng19-cookie-zone", "cookie:session", &["r"]);
        let headers = vec![(
            "Cookie".to_string(),
            "theme=dark; session=abc123; other=1".to_string(),
        )];
        assert_eq!(check(&cfg, &ctx("r", &headers)), LimitDecision::Allow);
        assert_eq!(check(&cfg, &ctx("r", &headers)), LimitDecision::Allow);
        assert!(matches!(
            check(&cfg, &ctx("r", &headers)),
            LimitDecision::Reject { .. }
        ));
    }

    #[test]
    fn user_key_uses_authenticated_identity_and_empty_user_bypasses() {
        let cfg = config("ng19-user-zone", "user", &["r"]);
        let mut request = ctx("r", &[]);
        request.user = Some("alice");
        assert_eq!(check(&cfg, &request), LimitDecision::Allow);
        assert_eq!(check(&cfg, &request), LimitDecision::Allow);
        assert!(matches!(
            check(&cfg, &request),
            LimitDecision::Reject { .. }
        ));
        request.user = Some("bob");
        assert_eq!(check(&cfg, &request), LimitDecision::Allow);
        request.user = None;
        for _ in 0..4 {
            assert_eq!(check(&cfg, &request), LimitDecision::Allow);
        }
    }

    #[test]
    fn nodelay_accepts_burst_then_rejects_without_waiting() {
        let cfg = with_options(
            config("ng19-nodelay", "ip", &["r"]),
            "r",
            Some(1),
            None,
            true,
        );
        assert_eq!(check(&cfg, &ctx("r", &[])), LimitDecision::Allow);
        assert_eq!(check(&cfg, &ctx("r", &[])), LimitDecision::Allow);
        assert!(matches!(
            check(&cfg, &ctx("r", &[])),
            LimitDecision::Reject { .. }
        ));
    }

    #[test]
    fn delay_zero_queues_excess_up_to_burst() {
        let mut cfg = config("ng19-delay", "ip", &["r"]);
        cfg.zones.get_mut("ng19-delay").unwrap().requests_per_second = 100;
        let cfg = with_options(cfg, "r", Some(1), Some(0), false);
        let now = Instant::now();
        assert_eq!(check_at(&cfg, &ctx("r", &[]), now), LimitDecision::Allow);
        let LimitDecision::Delay { wait, .. } = check_at(&cfg, &ctx("r", &[]), now) else {
            panic!("second request must be delayed");
        };
        assert_eq!(wait, Duration::from_millis(10));
        assert!(matches!(
            check_at(&cfg, &ctx("r", &[]), now),
            LimitDecision::Reject { .. }
        ));
    }

    #[test]
    fn delay_threshold_passes_first_n_excess_then_queues() {
        let mut cfg = config("ng19-delay-threshold", "ip", &["r"]);
        cfg.zones
            .get_mut("ng19-delay-threshold")
            .unwrap()
            .requests_per_second = 100;
        let cfg = with_options(cfg, "r", Some(2), Some(1), false);
        let now = Instant::now();
        assert_eq!(check_at(&cfg, &ctx("r", &[]), now), LimitDecision::Allow);
        assert_eq!(check_at(&cfg, &ctx("r", &[]), now), LimitDecision::Allow);
        assert!(matches!(
            check_at(&cfg, &ctx("r", &[]), now),
            LimitDecision::Delay { .. }
        ));
        assert!(matches!(
            check_at(&cfg, &ctx("r", &[]), now),
            LimitDecision::Reject { .. }
        ));
    }

    #[test]
    fn empty_header_key_is_not_accounted() {
        let cfg = with_options(
            config("ng19-empty-key", "header:x-api-key", &["r"]),
            "r",
            Some(1),
            None,
            true,
        );
        for _ in 0..4 {
            assert_eq!(check(&cfg, &ctx("r", &[])), LimitDecision::Allow);
        }
    }

    #[test]
    fn excess_decays_after_one_rate_interval() {
        let mut cfg = config("ng19-decay", "ip", &["r"]);
        cfg.zones.get_mut("ng19-decay").unwrap().burst = 0;
        let now = Instant::now();
        assert_eq!(check_at(&cfg, &ctx("r", &[]), now), LimitDecision::Allow);
        assert_eq!(
            check_at(&cfg, &ctx("r", &[]), now + Duration::from_secs(1)),
            LimitDecision::Allow
        );
    }

    #[test]
    fn combined_expression_uses_all_variables() {
        let cfg = config("ng19-expression", "expr:$remote_addr:$uri:$arg_x", &["r"]);
        let headers = vec![("Cookie".into(), "session=abc".into())];
        assert_eq!(check(&cfg, &ctx("r", &headers)), LimitDecision::Allow);
        assert_eq!(check(&cfg, &ctx("r", &headers)), LimitDecision::Allow);
        assert!(matches!(
            check(&cfg, &ctx("r", &headers)),
            LimitDecision::Reject { .. }
        ));
    }

    #[test]
    fn multiple_limits_use_custom_status_and_dry_run() {
        let mut cfg = config("ng19-multi-a", "ip", &[]);
        cfg.zones.insert(
            "ng19-multi-b".into(),
            RateLimitZoneConfig {
                key: "ip".into(),
                requests_per_second: 1,
                burst: 1,
            },
        );
        cfg.route_limits.insert(
            "r".into(),
            RateLimitRoutePolicy {
                limits: vec![
                    RateLimitLimit {
                        zone: "ng19-multi-a".into(),
                        burst: Some(1),
                        delay: None,
                        nodelay: true,
                    },
                    RateLimitLimit {
                        zone: "ng19-multi-b".into(),
                        burst: Some(1),
                        delay: None,
                        nodelay: true,
                    },
                ],
                status: 503,
                dry_run: false,
                log_level: "warn".into(),
            },
        );
        assert_eq!(check(&cfg, &ctx("r", &[])), LimitDecision::Allow);
        assert_eq!(check(&cfg, &ctx("r", &[])), LimitDecision::Allow);
        assert!(matches!(
            check(&cfg, &ctx("r", &[])),
            LimitDecision::Reject {
                status: 503,
                log_level: "warn",
                ..
            }
        ));
        cfg.route_limits.get_mut("r").unwrap().dry_run = true;
        assert!(matches!(
            check(&cfg, &ctx("r", &[])),
            LimitDecision::DryRun { status: 503, .. }
        ));
    }
}
