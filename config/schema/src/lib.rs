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
//! JSON Schema for ExyonQ IR v1/v2.

use exyonq_config_ir::{
    deterministic_endpoint_id, AdminEndpointState, Endpoint, EndpointFailoverPolicy,
    EndpointSelectionPolicy, CONFIG_VERSION_V1, CONFIG_VERSION_V2,
};

/// JSON Schema draft-07 document for native IR TOML (logical schema).
pub fn ir_json_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$id": "https://exyonq.org/schema/ir-v1.json",
        "title": "ExyonQConfigIR",
        "type": "object",
        "required": ["config_version", "server"],
        "properties": {
            "config_version": {
                "type": "integer",
                "enum": [CONFIG_VERSION_V1, CONFIG_VERSION_V2]
            },
            "include": {
                "type": "array",
                "items": { "type": "string" },
                "description": "config_version 2 only; fragment paths"
            },
            "server": {
                "type": "array",
                "minItems": 1,
                "items": { "$ref": "#/definitions/server" }
            },
            "route": {
                "type": "array",
                "items": { "$ref": "#/definitions/route" }
            },
            "upstream": {
                "type": "array",
                "items": { "$ref": "#/definitions/upstream" }
            },
            "modules": { "$ref": "#/definitions/modules" },
            "static": { "$ref": "#/definitions/static" }
        },
        "definitions": {
            "server": {
                "type": "object",
                "required": ["listen"],
                "properties": {
                    "listen": { "type": "string" },
                    "server_name": {
                        "oneOf": [
                            { "type": "string" },
                            { "type": "array", "items": { "type": "string" } }
                        ]
                    },
                    "routes": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "tls": {
                        "type": "object",
                        "required": ["cert", "key"],
                        "properties": {
                            "cert": { "type": "string" },
                            "key": { "type": "string" },
                            "acme": {
                                "type": "object",
                                "required": ["email", "domains"],
                                "properties": {
                                    "enabled": { "type": "boolean", "default": true },
                                    "email": { "type": "string", "format": "email" },
                                    "domains": {
                                        "type": "array",
                                        "items": { "type": "string" },
                                        "minItems": 1
                                    },
                                    "staging": { "type": "boolean", "default": false }
                                }
                            }
                        }
                    },
                    "http3_listen": { "type": "string" }
                }
            },
            "route": {
                "type": "object",
                "required": ["name", "match"],
                "properties": {
                    "name": { "type": "string" },
                    "match": {
                        "type": "object",
                        "required": ["path"],
                        "properties": {
                            "path": { "type": "string", "pattern": "^/" },
                            "host": { "type": "string" }
                        }
                    },
                    "upstream": { "type": "string" },
                    "root": { "type": "string" },
                    "index": { "type": "string" },
                    "redirect": {
                        "type": "object",
                        "required": ["location"],
                        "properties": {
                            "status": { "type": "integer" },
                            "location": { "type": "string" }
                        }
                    },
                    "rewrite": { "type": "string", "pattern": "^/" },
                    "location_rewrite": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["from", "to"],
                            "properties": {
                                "from": { "type": "string" },
                                "to": { "type": "string" }
                            }
                        }
                    }
                }
            },
            "upstream": {
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {
                        "type": "string",
                        "maxLength": 128,
                        "pattern": "^[A-Za-z0-9._-]+$"
                    },
                    "target": { "type": "string", "pattern": "^http://" },
                    "endpoints": {
                        "type": "array",
                        "maxItems": 256,
                        "items": {
                            "type": "object",
                            "required": ["address", "port"],
                            "properties": {
                                "id": {
                                    "type": "string",
                                    "maxLength": 128,
                                    "pattern": "^[A-Za-z0-9._-]+$"
                                },
                                "address": { "type": "string" },
                                "port": { "type": "integer", "minimum": 1, "maximum": 65535 },
                                "weight": {
                                    "type": "integer",
                                    "minimum": 0,
                                    "maximum": 4294967295u64
                                },
                                "priority": {
                                    "type": "integer",
                                    "minimum": 0,
                                    "maximum": 4294967295u64
                                },
                                "admin_state": {
                                    "type": "string",
                                    "enum": ["enabled", "disabled", "drain_requested"]
                                }
                            }
                        }
                    },
                    "selection_policy": {
                        "type": "string",
                        "enum": ["weighted_round_robin"]
                    },
                    "failover_policy": {
                        "type": "string",
                        "enum": ["priority_bands", "none"]
                    },
                    "timeout_ms": { "type": "integer", "minimum": 1 }
                },
                "not": {
                    "required": ["target", "endpoints"]
                }
            },
            "modules": {
                "type": "object",
                "properties": {
                    "metrics": {
                        "type": "object",
                        "properties": {
                            "enabled": { "type": "boolean" },
                            "path": { "type": "string" },
                            "health_path": { "type": "string" },
                            "scrape_bearer_token": {
                                "type": "string",
                                "description": "Required when metrics enabled and any server.listen is non-loopback (LA-CAP054-008)"
                            }
                        }
                    },
                    "compression": {
                        "type": "object",
                        "properties": {
                            "enabled": { "type": "boolean" },
                            "min_bytes": { "type": "integer" }
                        }
                    },
                    "ratelimit": {
                        "type": "object",
                        "properties": {
                            "enabled": { "type": "boolean" },
                            "requests_per_second": { "type": "integer" },
                            "burst": { "type": "integer" }
                        }
                    },
                    "real_ip": {
                        "type": "object",
                        "properties": {
                            "enabled": { "type": "boolean" },
                            "trusted": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Trusted proxy CIDRs/IPs (nginx set_real_ip_from). Required when enabled."
                            },
                            "header": {
                                "type": "string",
                                "description": "Forwarding chain header (nginx real_ip_header); default x-forwarded-for"
                            }
                        }
                    }
                }
            },
            "static": {
                "type": "object",
                "properties": {
                    "preload": {
                        "type": "object",
                        "properties": {
                            "max_file_bytes": {
                                "type": "integer",
                                "minimum": 0,
                                "description": "0 disables preload; default 33554432 (32 MiB)"
                            },
                            "max_total_bytes": {
                                "type": "integer",
                                "minimum": 0,
                                "description": "0 disables preload; default 268435456 (256 MiB)"
                            },
                            "max_entries": {
                                "type": "integer",
                                "minimum": 0,
                                "description": "0 disables preload; default 4096"
                            }
                        }
                    }
                }
            }
        }
    })
}

pub fn write_ir_schema(path: &std::path::Path) -> std::io::Result<()> {
    let schema = ir_json_schema();
    let pretty = serde_json::to_string_pretty(&schema).expect("schema serializes");
    std::fs::write(path, pretty)
}

/// Migrate v1 TOML string to v2 (sets config_version, preserves content).
pub fn migrate_v1_to_v2_toml(input: &str) -> Result<String, exyonq_config_ir::ConfigError> {
    let config = exyonq_config_ir::AppConfig::parse_str(input)?;
    let migrated = config.migrate_v1_to_v2();
    render_toml(&migrated)
}

fn admin_state_toml(state: AdminEndpointState) -> &'static str {
    match state {
        AdminEndpointState::Enabled => "enabled",
        AdminEndpointState::Disabled => "disabled",
        AdminEndpointState::DrainRequested => "drain_requested",
    }
}

fn render_toml(
    config: &exyonq_config_ir::AppConfig,
) -> Result<String, exyonq_config_ir::ConfigError> {
    let mut out = String::new();
    out.push_str(&format!("config_version = {}\n\n", config.config_version));
    for server in &config.servers {
        out.push_str("[[server]]\n");
        out.push_str(&format!("listen = {:?}\n", server.listen));
        if !server.server_name.is_empty() {
            let names = server.server_name.to_vec();
            if names.len() == 1 {
                out.push_str(&format!("server_name = {:?}\n", names[0]));
            } else {
                out.push_str("server_name = [");
                for (i, n) in names.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&format!("{n:?}"));
                }
                out.push_str("]\n");
            }
        }
        if !server.routes.is_empty() {
            out.push_str("routes = [");
            for (i, r) in server.routes.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("{r:?}"));
            }
            out.push_str("]\n");
        }
        out.push('\n');
    }
    for route in &config.routes {
        out.push_str("[[route]]\n");
        out.push_str(&format!("name = {:?}\n", route.name));
        out.push_str(&format!("match = {{ path = {:?}", route.r#match.path));
        if let Some(host) = &route.r#match.host {
            out.push_str(&format!(", host = {host:?}"));
        }
        out.push_str(" }\n");
        if let Some(upstream) = &route.upstream {
            out.push_str(&format!("upstream = {upstream:?}\n"));
        }
        if let Some(root) = &route.root {
            out.push_str(&format!("root = {:?}\n", root.display()));
        }
        if let Some(index) = &route.index {
            out.push_str(&format!("index = {index:?}\n"));
        }
        for lr in &route.location_rewrite {
            out.push_str("[[route.location_rewrite]]\n");
            out.push_str(&format!("from = {:?}\n", lr.from));
            out.push_str(&format!("to = {:?}\n", lr.to));
        }
        out.push('\n');
    }
    for upstream in config.upstreams.values() {
        out.push_str("[[upstream]]\n");
        out.push_str(&format!("name = {:?}\n", upstream.name));
        // TOML: every scalar of the [[upstream]] table must precede the
        // [[upstream.endpoints]] sub-tables or it lands inside the last one.
        out.push_str(&format!("timeout_ms = {}\n", upstream.timeout_ms));
        if upstream.max_connect_retries != 1 {
            out.push_str(&format!(
                "max_connect_retries = {}\n",
                upstream.max_connect_retries
            ));
        }
        if upstream.selection_policy != EndpointSelectionPolicy::default() {
            let policy = match upstream.selection_policy {
                EndpointSelectionPolicy::WeightedRoundRobin => "weighted_round_robin",
                EndpointSelectionPolicy::LeastConn => "least_conn",
                EndpointSelectionPolicy::PowerOfTwo => "power_of_two",
                EndpointSelectionPolicy::ConsistentHash => "consistent_hash",
                EndpointSelectionPolicy::CookieAffinity => "cookie_affinity",
                EndpointSelectionPolicy::SourceAffinity => "source_affinity",
                EndpointSelectionPolicy::HeaderAffinity => "header_affinity",
            };
            out.push_str(&format!("selection_policy = {policy:?}\n"));
        }
        if let Some(cookie) = &upstream.sticky_cookie {
            out.push_str(&format!("sticky_cookie = {cookie:?}\n"));
        }
        if let Some(header) = &upstream.sticky_header {
            out.push_str(&format!("sticky_header = {header:?}\n"));
        }
        // NG-09: emit refresh interval only when periodic re-resolve is on.
        if upstream.dns_refresh_ms != 0 {
            out.push_str(&format!("dns_refresh_ms = {}\n", upstream.dns_refresh_ms));
        }
        if upstream.failover_policy != EndpointFailoverPolicy::default() {
            let policy = match upstream.failover_policy {
                EndpointFailoverPolicy::PriorityBands => "priority_bands",
                EndpointFailoverPolicy::None => "none",
            };
            out.push_str(&format!("failover_policy = {policy:?}\n"));
        }
        // SERIALIZATION_POLICY: `target` sugar only when the sole endpoint is
        // all-default — weight/priority/admin_state/passive-health params and
        // a custom endpoint id must not be silently dropped by the shorthand.
        let eps = upstream.endpoint_set.endpoints();
        let legacy_single = eps.len() == 1
            && eps[0].weight == Endpoint::DEFAULT_WEIGHT
            && eps[0].priority == Endpoint::DEFAULT_PRIORITY
            && eps[0].admin_state == AdminEndpointState::Enabled
            && eps[0].max_fails == Endpoint::DEFAULT_MAX_FAILS
            && eps[0].fail_timeout_ms == Endpoint::DEFAULT_FAIL_TIMEOUT_MS
            && eps[0].slow_start_ms == Endpoint::DEFAULT_SLOW_START_MS
            && eps[0].endpoint_id
                == deterministic_endpoint_id(
                    &eps[0].address,
                    eps[0].port,
                    eps[0].weight,
                    eps[0].priority,
                );
        if upstream.endpoint_set.len() <= 1 && !upstream.target.is_empty() && legacy_single {
            out.push_str(&format!("target = {:?}\n", upstream.target));
        }
        if upstream.health_check.enabled {
            let hc = &upstream.health_check;
            out.push_str(&format!(
                "health_check = {{ enabled = true, interval_ms = {}, timeout_ms = {}, path = {:?}, healthy_threshold = {}, unhealthy_threshold = {}",
                hc.interval_ms, hc.timeout_ms, hc.path, hc.healthy_threshold, hc.unhealthy_threshold
            ));
            if !hc.expect_status.is_empty() {
                out.push_str(&format!(", expect_status = {:?}", hc.expect_status));
            }
            if let Some(b) = &hc.expect_body {
                out.push_str(&format!(", expect_body = {:?}", b));
            }
            if let Some(h) = &hc.expect_header {
                out.push_str(&format!(", expect_header = {:?}", h));
            }
            out.push_str(" }\n");
        }
        if !(upstream.endpoint_set.len() <= 1 && !upstream.target.is_empty() && legacy_single) {
            for ep in eps {
                out.push_str("[[upstream.endpoints]]\n");
                out.push_str(&format!("id = {:?}\n", ep.endpoint_id.as_str()));
                out.push_str(&format!("address = {:?}\n", ep.address.as_host_str()));
                out.push_str(&format!("port = {}\n", ep.port));
                out.push_str(&format!("weight = {}\n", ep.weight));
                out.push_str(&format!("priority = {}\n", ep.priority));
                if ep.admin_state != AdminEndpointState::Enabled {
                    out.push_str(&format!(
                        "admin_state = {:?}\n",
                        admin_state_toml(ep.admin_state)
                    ));
                }
                if ep.max_fails != Endpoint::DEFAULT_MAX_FAILS {
                    out.push_str(&format!("max_fails = {}\n", ep.max_fails));
                }
                if ep.fail_timeout_ms != Endpoint::DEFAULT_FAIL_TIMEOUT_MS {
                    out.push_str(&format!("fail_timeout_ms = {}\n", ep.fail_timeout_ms));
                }
                if ep.slow_start_ms != Endpoint::DEFAULT_SLOW_START_MS {
                    out.push_str(&format!("slow_start_ms = {}\n", ep.slow_start_ms));
                }
            }
        }
        out.push('\n');
    }
    // NG-11: trusted-proxy client-IP resolution.
    let rip = &config.modules.real_ip;
    if rip.enabled || !rip.trusted.is_empty() {
        out.push_str("[modules.real_ip]\n");
        out.push_str(&format!("enabled = {}\n", rip.enabled));
        out.push_str(&format!("trusted = {:?}\n", rip.trusted));
        if rip.header != "x-forwarded-for" {
            out.push_str(&format!("header = {:?}\n", rip.header));
        }
        out.push('\n');
    }
    Ok(out)
}

/// Render validated IR as TOML (used by migrator and Serverfile lowering).
pub fn render_toml_public(
    config: &exyonq_config_ir::AppConfig,
) -> Result<String, exyonq_config_ir::ConfigError> {
    render_toml(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_required_fields() {
        let schema = ir_json_schema();
        assert!(schema.get("definitions").is_some());
    }

    #[test]
    fn migrate_v1_to_v2_bumps_version() {
        let input = include_str!("../../../tests/fixtures/minimal.toml");
        let out = migrate_v1_to_v2_toml(input).unwrap();
        assert!(out.starts_with("config_version = 2"));
    }

    #[test]
    fn render_keeps_nondefault_passive_fields() {
        // NG-08/F6: passive-health params must survive render — a tuned
        // endpoint must not collapse into the `target` shorthand.
        let input = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
upstream = "b"
[[upstream]]
name = "b"
endpoints = [
    { address = "10.0.0.2", port = 8080, max_fails = 3, fail_timeout_ms = 5_000, slow_start_ms = 30_000 },
]
"#;
        let cfg = exyonq_config_ir::AppConfig::parse_str(input).unwrap();
        let out = render_toml(&cfg).unwrap();
        assert!(
            out.contains("max_fails = 3"),
            "render must emit max_fails: {out}"
        );
        assert!(out.contains("fail_timeout_ms = 5000"));
        assert!(out.contains("slow_start_ms = 30000"));
        assert!(
            !out.contains("target = "),
            "tuned endpoint must not render as target sugar"
        );
        // Round-trip: the rendered TOML re-parses with the same values.
        let cfg2 = exyonq_config_ir::AppConfig::parse_str(&out).unwrap();
        let ep = &cfg2.upstreams["b"].endpoint_set.endpoints()[0];
        assert_eq!(ep.max_fails, 3);
        assert_eq!(ep.fail_timeout_ms, 5_000);
        assert_eq!(ep.slow_start_ms, 30_000);
    }

    #[test]
    fn render_keeps_failover_policy() {
        // NG-08 audit N2: failover_policy must survive render — silently
        // dropping it turns "none" back into the default banded failover.
        let input = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
upstream = "b"
[[upstream]]
name = "b"
failover_policy = "none"
target = "http://10.0.0.2:8080"
"#;
        let cfg = exyonq_config_ir::AppConfig::parse_str(input).unwrap();
        let out = render_toml(&cfg).unwrap();
        assert!(
            out.contains("failover_policy = \"none\""),
            "render must emit failover_policy: {out}"
        );
        let cfg2 = exyonq_config_ir::AppConfig::parse_str(&out).unwrap();
        assert_eq!(
            cfg2.upstreams["b"].failover_policy,
            exyonq_config_ir::EndpointFailoverPolicy::None
        );
    }

    #[test]
    fn render_keeps_health_expect_fields() {
        // NG-10: expect_* must round-trip through render (migrator path).
        let input = r#"
config_version = 2
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
upstream = "b"
[[upstream]]
name = "b"
health_check = { enabled = true, interval_ms = 500, timeout_ms = 200, path = "/health", healthy_threshold = 1, unhealthy_threshold = 2, expect_status = [200, 204], expect_body = "READY", expect_header = "X-Health: pass" }
endpoints = [ { address = "10.0.0.2", port = 8080 } ]
"#;
        let cfg = exyonq_config_ir::AppConfig::parse_str(input).unwrap();
        let out = render_toml(&cfg).unwrap();
        let cfg2 = exyonq_config_ir::AppConfig::parse_str(&out).unwrap();
        let hc = &cfg2.upstreams["b"].health_check;
        assert_eq!(hc.expect_status, vec![200, 204]);
        assert_eq!(hc.expect_body.as_deref(), Some("READY"));
        assert_eq!(hc.expect_header.as_deref(), Some("X-Health: pass"));
    }

    #[test]
    fn render_keeps_explicit_endpoint_id() {
        // NG-08 audit N3: a custom endpoint id must not collapse into the
        // `target` shorthand — re-parse would mint a deterministic id and
        // silently change endpoint identity (fingerprints, cookie pins).
        let input = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
upstream = "b"
[[upstream]]
name = "b"
endpoints = [
    { id = "web1", address = "10.0.0.2", port = 8080 },
]
"#;
        let cfg = exyonq_config_ir::AppConfig::parse_str(input).unwrap();
        let out = render_toml(&cfg).unwrap();
        assert!(
            !out.contains("target = "),
            "custom-id endpoint must not render as target sugar: {out}"
        );
        assert!(out.contains("id = \"web1\""), "custom id preserved: {out}");
        let cfg2 = exyonq_config_ir::AppConfig::parse_str(&out).unwrap();
        assert_eq!(
            cfg2.upstreams["b"].endpoint_set.endpoints()[0]
                .endpoint_id
                .as_str(),
            "web1"
        );
    }
}
