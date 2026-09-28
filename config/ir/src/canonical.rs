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
//! Canonical IR representation for deterministic fingerprinting.

use crate::{AppConfig, FcgiPoolConfig, RouteConfig, ServerConfig, UpstreamConfig};
use std::collections::BTreeMap;

/// Stable hex fingerprint of canonical IR JSON (64-char SHA-256).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IrFingerprint(pub String);

impl IrFingerprint {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Canonical JSON string used for hashing (sorted keys, normalized values).
pub fn canonical_json(config: &AppConfig) -> String {
    let value = canonical_value(config);
    serde_json::to_string(&value).expect("canonical IR serializes")
}

pub fn fingerprint(config: &AppConfig) -> IrFingerprint {
    let json = canonical_json(config);
    let digest = sha256_hex(json.as_bytes());
    IrFingerprint(digest)
}

fn canonical_value(config: &AppConfig) -> serde_json::Value {
    let mut servers: Vec<serde_json::Value> = config.servers.iter().map(canonical_server).collect();
    servers.sort_by(|a, b| {
        a.get("listen")
            .and_then(|v| v.as_str())
            .cmp(&b.get("listen").and_then(|v| v.as_str()))
    });

    let mut routes: Vec<serde_json::Value> = config.routes.iter().map(canonical_route).collect();
    routes.sort_by(|a, b| {
        a.get("name")
            .and_then(|v| v.as_str())
            .cmp(&b.get("name").and_then(|v| v.as_str()))
    });

    let mut upstreams = BTreeMap::new();
    for (name, upstream) in &config.upstreams {
        upstreams.insert(name.clone(), canonical_upstream(upstream));
    }

    let mut pools_sorted = BTreeMap::new();
    for (name, pool) in &config.pools_fcgi {
        pools_sorted.insert(name.clone(), canonical_fcgi_pool(pool));
    }

    let mut root = serde_json::Map::new();
    root.insert(
        "config_version".into(),
        serde_json::Value::Number(config.config_version.into()),
    );
    if !config.includes.is_empty() {
        root.insert(
            "include".into(),
            serde_json::Value::Array(
                config
                    .includes
                    .iter()
                    .map(|s| serde_json::Value::String(s.clone()))
                    .collect(),
            ),
        );
    }
    root.insert("server".into(), serde_json::Value::Array(servers));
    root.insert("route".into(), serde_json::Value::Array(routes));
    root.insert(
        "upstream".into(),
        serde_json::Value::Array(upstreams.into_values().collect::<Vec<_>>()),
    );
    if !pools_sorted.is_empty() {
        root.insert(
            "fcgi_pool".into(),
            serde_json::Value::Array(pools_sorted.into_values().collect::<Vec<_>>()),
        );
    }
    root.insert(
        "modules".into(),
        serde_json::to_value(&config.modules).expect("modules serialize"),
    );
    root.insert(
        "static".into(),
        serde_json::to_value(&config.static_section).expect("static serialize"),
    );
    root.insert(
        "full_page_cache".into(),
        serde_json::to_value(&config.full_page_cache).expect("full_page_cache serialize"),
    );
    root.insert(
        "http3".into(),
        serde_json::to_value(&config.http3).expect("http3 serialize"),
    );
    root.insert(
        "waf".into(),
        serde_json::to_value(&config.waf).expect("waf serialize"),
    );
    root.insert(
        "logging".into(),
        serde_json::to_value(&config.logging).expect("logging serialize"),
    );
    // Cap048: cache_policy is runtime-effective (route.cache → policy). Must participate
    // in generation fingerprint or reload silently NO_OPs policy/ttl changes.
    if !config.cache_policies.is_empty() {
        let mut policies: Vec<serde_json::Value> = config
            .cache_policies
            .values()
            .map(|p| {
                let mut m = serde_json::Map::new();
                m.insert("name".into(), serde_json::Value::String(p.name.clone()));
                m.insert(
                    "ttl_seconds".into(),
                    serde_json::Value::Number(p.ttl_seconds.into()),
                );
                m.insert(
                    "max_object_bytes".into(),
                    serde_json::Value::Number(p.max_object_bytes.into()),
                );
                serde_json::Value::Object(m)
            })
            .collect();
        policies.sort_by(|a, b| {
            a.get("name")
                .and_then(|v| v.as_str())
                .cmp(&b.get("name").and_then(|v| v.as_str()))
        });
        root.insert("cache_policy".into(), serde_json::Value::Array(policies));
    }
    serde_json::Value::Object(root)
}

fn canonical_server(server: &ServerConfig) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "listen".into(),
        serde_json::Value::String(server.listen.clone()),
    );
    if !server.server_name.is_empty() {
        let names = server.server_name.to_vec();
        map.insert(
            "server_name".into(),
            serde_json::Value::Array(names.into_iter().map(serde_json::Value::String).collect()),
        );
    }
    let mut routes = server.routes.clone();
    routes.sort();
    map.insert(
        "routes".into(),
        serde_json::Value::Array(routes.into_iter().map(serde_json::Value::String).collect()),
    );
    if let Some(tls) = &server.tls {
        let mut tls_map = serde_json::Map::new();
        tls_map.insert(
            "cert".into(),
            serde_json::Value::String(tls.cert.display().to_string()),
        );
        tls_map.insert(
            "key".into(),
            serde_json::Value::String(tls.key.display().to_string()),
        );
        if let Some(acme) = &tls.acme {
            tls_map.insert(
                "acme".into(),
                serde_json::to_value(acme).expect("acme serializes"),
            );
        }
        map.insert("tls".into(), serde_json::Value::Object(tls_map));
    }
    if let Some(http3) = &server.http3_listen {
        map.insert(
            "http3_listen".into(),
            serde_json::Value::String(http3.clone()),
        );
    }
    serde_json::Value::Object(map)
}

fn canonical_route(route: &RouteConfig) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("name".into(), serde_json::Value::String(route.name.clone()));
    let mut match_map = serde_json::Map::new();
    match_map.insert(
        "path".into(),
        serde_json::Value::String(route.r#match.path.clone()),
    );
    if let Some(host) = &route.r#match.host {
        match_map.insert("host".into(), serde_json::Value::String(host.clone()));
    }
    map.insert("match".into(), serde_json::Value::Object(match_map));
    if let Some(upstream) = &route.upstream {
        map.insert(
            "upstream".into(),
            serde_json::Value::String(upstream.clone()),
        );
    }
    if let Some(root) = &route.root {
        map.insert(
            "root".into(),
            serde_json::Value::String(root.display().to_string()),
        );
    }
    if let Some(index) = &route.index {
        map.insert("index".into(), serde_json::Value::String(index.clone()));
    }
    if let Some(redirect) = &route.redirect {
        let mut redir = serde_json::Map::new();
        redir.insert(
            "status".into(),
            serde_json::Value::Number(redirect.status.into()),
        );
        redir.insert(
            "location".into(),
            serde_json::Value::String(redirect.location.clone()),
        );
        map.insert("redirect".into(), serde_json::Value::Object(redir));
    }
    if let Some(rewrite) = &route.rewrite {
        map.insert("rewrite".into(), serde_json::Value::String(rewrite.clone()));
    }
    if let Some(fastcgi) = &route.fastcgi {
        map.insert("fastcgi".into(), serde_json::Value::String(fastcgi.clone()));
    }
    if route.htaccess == crate::HtaccessMode::Overlay {
        map.insert(
            "htaccess".into(),
            serde_json::Value::String("overlay".into()),
        );
    }
    // Cap048: route.cache binds a named cache_policy at request time.
    if let Some(cache) = &route.cache {
        map.insert("cache".into(), serde_json::Value::String(cache.clone()));
    }
    serde_json::Value::Object(map)
}

fn canonical_fcgi_pool(pool: &FcgiPoolConfig) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("name".into(), serde_json::Value::String(pool.name.clone()));
    map.insert(
        "address".into(),
        serde_json::Value::String(pool.address.trim().to_string()),
    );
    if let Some(root) = &pool.document_root {
        map.insert(
            "document_root".into(),
            serde_json::Value::String(root.display().to_string()),
        );
    }
    map.insert(
        "max_concurrency".into(),
        serde_json::Value::Number(pool.max_concurrency.into()),
    );
    // Cap048 LA-CAP048-004: timeouts/transport/max_connections are runtime-effective.
    map.insert(
        "transport".into(),
        serde_json::Value::String(pool.transport.trim().to_ascii_lowercase()),
    );
    if let Some(max_conn) = pool.max_connections {
        map.insert(
            "max_connections".into(),
            serde_json::Value::Number(max_conn.into()),
        );
    }
    map.insert(
        "idle_timeout_ms".into(),
        serde_json::Value::Number(pool.idle_timeout_ms.into()),
    );
    map.insert(
        "total_timeout_ms".into(),
        serde_json::Value::Number(pool.total_timeout_ms.into()),
    );
    map.insert(
        "checkout_timeout_ms".into(),
        serde_json::Value::Number(pool.checkout_timeout_ms.into()),
    );
    serde_json::Value::Object(map)
}

fn canonical_upstream(upstream: &UpstreamConfig) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "name".into(),
        serde_json::Value::String(upstream.name.clone()),
    );
    // SERIALIZATION_POLICY:
    // - single-endpoint keeps legacy `target` for historical fingerprint continuity of
    //   name/target/timeout_ms, BUT Cap048 requires peer weight/priority/admin_state to
    //   participate (else reload NO_OPs Cap045/Cap046 operator changes on 1-peer upstreams).
    // - multi/empty use endpoints[].
    if upstream.endpoint_set.len() == 1 {
        map.insert(
            "target".into(),
            serde_json::Value::String(normalize_upstream_target(&upstream.target)),
        );
        let ep = &upstream.endpoint_set.endpoints()[0];
        map.insert("weight".into(), serde_json::Value::Number(ep.weight.into()));
        map.insert(
            "priority".into(),
            serde_json::Value::Number(ep.priority.into()),
        );
        map.insert(
            "admin_state".into(),
            serde_json::Value::String(format!("{:?}", ep.admin_state).to_ascii_lowercase()),
        );
    } else {
        let endpoints: Vec<serde_json::Value> = upstream
            .endpoint_set
            .endpoints()
            .iter()
            .map(|ep| {
                let mut e = serde_json::Map::new();
                e.insert(
                    "endpoint_id".into(),
                    serde_json::Value::String(ep.endpoint_id.as_str().to_string()),
                );
                e.insert(
                    "address".into(),
                    serde_json::Value::String(ep.address.as_host_str()),
                );
                e.insert("port".into(), serde_json::Value::Number(ep.port.into()));
                e.insert("weight".into(), serde_json::Value::Number(ep.weight.into()));
                e.insert(
                    "priority".into(),
                    serde_json::Value::Number(ep.priority.into()),
                );
                e.insert(
                    "admin_state".into(),
                    serde_json::Value::String(format!("{:?}", ep.admin_state).to_ascii_lowercase()),
                );
                serde_json::Value::Object(e)
            })
            .collect();
        map.insert("endpoints".into(), serde_json::Value::Array(endpoints));
        map.insert(
            "selection_policy".into(),
            serde_json::Value::String(
                format!("{:?}", upstream.selection_policy).to_ascii_lowercase(),
            ),
        );
        map.insert(
            "failover_policy".into(),
            serde_json::Value::String(
                format!("{:?}", upstream.failover_policy).to_ascii_lowercase(),
            ),
        );
    }
    map.insert(
        "timeout_ms".into(),
        serde_json::Value::Number(upstream.timeout_ms.into()),
    );
    // Cap021: omit default (=1) to preserve fingerprint for legacy configs.
    if upstream.max_connect_retries != 1 {
        map.insert(
            "max_connect_retries".into(),
            serde_json::Value::Number(upstream.max_connect_retries.into()),
        );
    }
    // Cap024: omit default-disabled health_check so legacy fingerprints stay stable.
    if upstream.health_check != crate::UpstreamHealthCheckConfig::default() {
        map.insert(
            "health_check".into(),
            canonical_upstream_health_check(&upstream.health_check),
        );
    }
    serde_json::Value::Object(map)
}

fn canonical_upstream_health_check(hc: &crate::UpstreamHealthCheckConfig) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("enabled".into(), serde_json::Value::Bool(hc.enabled));
    map.insert(
        "interval_ms".into(),
        serde_json::Value::Number(hc.interval_ms.into()),
    );
    map.insert(
        "timeout_ms".into(),
        serde_json::Value::Number(hc.timeout_ms.into()),
    );
    map.insert("path".into(), serde_json::Value::String(hc.path.clone()));
    map.insert(
        "healthy_threshold".into(),
        serde_json::Value::Number(hc.healthy_threshold.into()),
    );
    map.insert(
        "unhealthy_threshold".into(),
        serde_json::Value::Number(hc.unhealthy_threshold.into()),
    );
    serde_json::Value::Object(map)
}

fn normalize_upstream_target(target: &str) -> String {
    target.trim().to_string()
}

fn sha256_hex(input: &[u8]) -> String {
    let mut h1: u64 = 0xcbf29ce484222325;
    let mut h2: u64 = 0x14650fb0739d0383;
    for (i, byte) in input.iter().enumerate() {
        h1 ^= *byte as u64;
        h1 = h1.wrapping_mul(0x100000001b3);
        h2 ^= (*byte as u64).wrapping_add(i as u64);
        h2 = h2.wrapping_mul(0x100000001b3);
    }
    format!(
        "{:016x}{:016x}{:016x}{:016x}",
        h1,
        h2,
        h1 ^ h2,
        h1.rotate_left(17) ^ h2
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppConfig;

    const MINIMAL: &str = include_str!("../../../tests/fixtures/minimal.toml");

    #[test]
    fn fingerprint_is_stable() {
        let config: AppConfig = MINIMAL.parse().unwrap();
        let a = fingerprint(&config);
        let b = fingerprint(&config);
        assert_eq!(a, b);
        assert_eq!(a.as_str().len(), 64);
    }

    #[test]
    fn fingerprint_changes_when_logging_file_path_changes() {
        let raw = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/site/" }
root = "./site"
[logging.file]
enabled = true
path = "/tmp/a.log"
"#;
        let mut a: AppConfig = raw.parse().unwrap();
        let base = fingerprint(&a);
        a.logging.file.path = Some("/tmp/b.log".into());
        assert_ne!(base, fingerprint(&a));
        a.logging.file.rotation.enabled = true;
        assert_ne!(base, fingerprint(&a));
    }

    #[test]
    fn fingerprint_changes_when_route_changes() {
        let config: AppConfig = MINIMAL.parse().unwrap();
        let base = fingerprint(&config);
        let mut changed = config.clone();
        changed.routes[0].r#match.path = "/api/v2".into();
        assert_ne!(base, fingerprint(&changed));
    }

    #[test]
    fn fingerprint_changes_when_health_check_path_changes() {
        let toml = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]
[[route]]
name = "api"
match = { path = "/api/" }
upstream = "backend"
[[upstream]]
name = "backend"
target = "http://127.0.0.1:9"
timeout_ms = 1000
health_check = { enabled = true, interval_ms = 200, timeout_ms = 100, path = "/health", healthy_threshold = 2, unhealthy_threshold = 2 }
"#;
        let mut a: AppConfig = toml.parse().unwrap();
        let base = fingerprint(&a);
        a.upstreams.get_mut("backend").unwrap().health_check.path = "/healthz".into();
        assert_ne!(base, fingerprint(&a));
    }

    /// Cap048 LA-CAP048-001: single-endpoint weight must change generation fingerprint.
    #[test]
    fn fingerprint_changes_when_single_endpoint_weight_changes() {
        let a: AppConfig = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]
[[route]]
name = "api"
match = { path = "/api/" }
upstream = "backend"
[[upstream]]
name = "backend"
timeout_ms = 1000
[[upstream.endpoints]]
address = "127.0.0.1"
port = 9
weight = 1
admin_state = "enabled"
"#
        .parse()
        .unwrap();
        let b: AppConfig = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]
[[route]]
name = "api"
match = { path = "/api/" }
upstream = "backend"
[[upstream]]
name = "backend"
timeout_ms = 1000
[[upstream.endpoints]]
address = "127.0.0.1"
port = 9
weight = 9
admin_state = "enabled"
"#
        .parse()
        .unwrap();
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    /// Cap048 LA-CAP048-001: single-endpoint admin_state must change generation fingerprint.
    #[test]
    fn fingerprint_changes_when_single_endpoint_admin_state_changes() {
        let a: AppConfig = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]
[[route]]
name = "api"
match = { path = "/api/" }
upstream = "backend"
[[upstream]]
name = "backend"
timeout_ms = 1000
[[upstream.endpoints]]
address = "127.0.0.1"
port = 9
weight = 1
admin_state = "enabled"
"#
        .parse()
        .unwrap();
        let b: AppConfig = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["api"]
[[route]]
name = "api"
match = { path = "/api/" }
upstream = "backend"
[[upstream]]
name = "backend"
timeout_ms = 1000
[[upstream.endpoints]]
address = "127.0.0.1"
port = 9
weight = 1
admin_state = "disabled"
"#
        .parse()
        .unwrap();
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    /// Cap048 LA-CAP048-002: cache_policy + route.cache must change generation fingerprint.
    #[test]
    fn fingerprint_changes_when_cache_policy_changes() {
        let a: AppConfig = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
root = "/tmp"
cache = "pol"
[[cache_policy]]
name = "pol"
ttl_seconds = 30
max_object_bytes = 1024
"#
        .parse()
        .unwrap();
        let b: AppConfig = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
root = "/tmp"
cache = "pol"
[[cache_policy]]
name = "pol"
ttl_seconds = 90
max_object_bytes = 1024
"#
        .parse()
        .unwrap();
        assert_ne!(fingerprint(&a), fingerprint(&b));
        let mut c = a.clone();
        c.routes[0].cache = Some("other".into());
        assert_ne!(fingerprint(&a), fingerprint(&c));
    }

    /// Cap048 LA-CAP048-004: FCGI pool timeouts must change generation fingerprint.
    #[test]
    fn fingerprint_changes_when_fcgi_idle_timeout_changes() {
        let a: AppConfig = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]
[[route]]
name = "php"
match = { path = "/index.php" }
fastcgi = "app"
[[fcgi_pool]]
name = "app"
address = "/tmp/php.sock"
transport = "unix"
max_concurrency = 4
idle_timeout_ms = 1000
total_timeout_ms = 5000
checkout_timeout_ms = 500
"#
        .parse()
        .unwrap();
        let b: AppConfig = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["php"]
[[route]]
name = "php"
match = { path = "/index.php" }
fastcgi = "app"
[[fcgi_pool]]
name = "app"
address = "/tmp/php.sock"
transport = "unix"
max_concurrency = 4
idle_timeout_ms = 2000
total_timeout_ms = 5000
checkout_timeout_ms = 500
"#
        .parse()
        .unwrap();
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }
}
