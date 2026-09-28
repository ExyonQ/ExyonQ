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

use exyonq_config_ir::{CONFIG_VERSION_V1, CONFIG_VERSION_V2};

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
                    "rewrite": { "type": "string", "pattern": "^/" }
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
                    },
                    "encoding_cache": {
                        "type": "object",
                        "description": "ADR-046 Cap067 static encoding cache (default off)",
                        "properties": {
                            "enabled": { "type": "boolean", "default": false },
                            "cache_dir": {
                                "type": "string",
                                "default": "/var/cache/exyonq/static-encoding"
                            },
                            "level": { "type": "integer", "minimum": 0, "maximum": 11, "default": 6 },
                            "min_bytes": { "type": "integer", "minimum": 0, "default": 300 },
                            "max_bytes": { "type": "integer", "minimum": 1, "default": 10485760 },
                            "gzip": { "type": "boolean", "default": true },
                            "brotli": { "type": "boolean", "default": false }
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
        out.push('\n');
    }
    for upstream in config.upstreams.values() {
        out.push_str("[[upstream]]\n");
        out.push_str(&format!("name = {:?}\n", upstream.name));
        // SERIALIZATION_POLICY: prefer legacy `target` for single-endpoint round-trip sugar.
        if upstream.endpoint_set.len() <= 1 && !upstream.target.is_empty() {
            out.push_str(&format!("target = {:?}\n", upstream.target));
        } else {
            for ep in upstream.endpoint_set.endpoints() {
                out.push_str("[[upstream.endpoints]]\n");
                out.push_str(&format!("id = {:?}\n", ep.endpoint_id.as_str()));
                out.push_str(&format!("address = {:?}\n", ep.address.as_host_str()));
                out.push_str(&format!("port = {}\n", ep.port));
                out.push_str(&format!("weight = {}\n", ep.weight));
                out.push_str(&format!("priority = {}\n", ep.priority));
            }
        }
        out.push_str(&format!("timeout_ms = {}\n\n", upstream.timeout_ms));
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
}
