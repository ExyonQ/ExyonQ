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
//! `config dump-ir` — load/validate AppConfig and emit canonical IR.

use crate::load::load_app_config;
use crate::output::{sanitize_control_chars, CliExit, ToolResult};
use exyonq_config_ir::{canonical_json, AppConfig, HtaccessMode};
use std::path::Path;

/// Output body format for `dump-ir`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpIrFormat {
    Toml,
    Json,
}

/// Load + validate config, then print canonical IR to stdout (errors → non-zero).
pub fn dump_ir(path: &Path, format: DumpIrFormat) -> ToolResult {
    let cfg = match load_app_config(path) {
        Ok(c) => c,
        Err(err) => {
            return ToolResult {
                exit: CliExit::DiagnosticError,
                stdout: String::new(),
                stderr: format!("error[EXY-CONFIG-0004]: {}\n", sanitize_control_chars(&err)),
            };
        }
    };

    match format {
        DumpIrFormat::Json => match pretty_canonical_json(&cfg) {
            Ok(body) => ToolResult {
                exit: CliExit::Ok,
                stdout: body,
                stderr: String::new(),
            },
            Err(err) => ToolResult {
                exit: CliExit::DiagnosticError,
                stdout: String::new(),
                stderr: format!("error[EXY-CONFIG-0004]: {}\n", sanitize_control_chars(&err)),
            },
        },
        DumpIrFormat::Toml => ToolResult {
            exit: CliExit::Ok,
            stdout: render_canonical_toml(&cfg),
            stderr: String::new(),
        },
    }
}

fn pretty_canonical_json(cfg: &AppConfig) -> Result<String, String> {
    let compact = canonical_json(cfg);
    let value: serde_json::Value =
        serde_json::from_str(&compact).map_err(|e| format!("canonical JSON parse: {e}"))?;
    serde_json::to_string_pretty(&value).map_err(|e| format!("canonical JSON emit: {e}"))
}

/// Deterministic TOML IR emission for dump-ir (round-trips common product shapes).
fn render_canonical_toml(cfg: &AppConfig) -> String {
    let mut out = String::new();
    out.push_str(&format!("config_version = {}\n\n", cfg.config_version));

    for server in &cfg.servers {
        out.push_str("[[server]]\n");
        out.push_str(&format!("listen = {:?}\n", server.listen));
        let names = server.server_name.to_vec();
        if names.len() == 1 {
            out.push_str(&format!("server_name = {:?}\n", names[0]));
        } else if names.len() > 1 {
            out.push_str("server_name = [");
            for (i, n) in names.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("{n:?}"));
            }
            out.push_str("]\n");
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
        if let Some(tls) = &server.tls {
            out.push_str(&format!(
                "tls = {{ cert = {:?}, key = {:?} }}\n",
                tls.cert.display(),
                tls.key.display()
            ));
        }
        if let Some(h3) = &server.http3_listen {
            out.push_str(&format!("http3_listen = {h3:?}\n"));
        }
        out.push('\n');
    }

    for route in &cfg.routes {
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
        if let Some(redirect) = &route.redirect {
            out.push_str(&format!(
                "redirect = {{ status = {}, location = {:?} }}\n",
                redirect.status, redirect.location
            ));
        }
        if let Some(rewrite) = &route.rewrite {
            match rewrite {
                exyonq_config_ir::RewriteConfig::Literal(path) => {
                    out.push_str(&format!("rewrite = {path:?}\n"));
                }
                exyonq_config_ir::RewriteConfig::Rule(rule) => {
                    out.push_str(&format!(
                        "rewrite = {{ pattern = {:?}, replacement = {:?}, flag = {:?} }}\n",
                        rule.pattern, rule.replacement, rule.flag
                    ));
                }
            }
        }
        if let Some(fastcgi) = &route.fastcgi {
            out.push_str(&format!("fastcgi = {fastcgi:?}\n"));
        }
        if route.htaccess != HtaccessMode::Off {
            let mode = match route.htaccess {
                HtaccessMode::Off => "off",
                HtaccessMode::Overlay => "overlay",
            };
            out.push_str(&format!("htaccess = {mode:?}\n"));
        }
        if let Some(cache) = &route.cache {
            out.push_str(&format!("cache = {cache:?}\n"));
        }
        if let Some(ch) = &route.cache_headers {
            for (mime, hdr) in &ch.by_mime {
                out.push_str(&format!("[route.cache_headers.by_mime.{mime:?}]\n"));
                out.push_str(&format!("max_age = {}\n", hdr.max_age));
                if hdr.immutable {
                    out.push_str("immutable = true\n");
                }
            }
        }
        for h in &route.response_headers {
            out.push_str("[[route.response_headers]]\n");
            out.push_str(&format!("name = {:?}\n", h.name));
            out.push_str(&format!("value = {:?}\n", h.value));
            if h.always {
                out.push_str("always = true\n");
            }
        }
        out.push('\n');
    }

    let mut upstream_names: Vec<_> = cfg.upstreams.keys().collect();
    upstream_names.sort();
    for name in upstream_names {
        let upstream = &cfg.upstreams[name];
        out.push_str("[[upstream]]\n");
        out.push_str(&format!("name = {:?}\n", upstream.name));
        if !upstream.target.is_empty() {
            out.push_str(&format!("target = {:?}\n", upstream.target));
        }
        out.push_str(&format!("timeout_ms = {}\n\n", upstream.timeout_ms));
    }

    let mut pool_names: Vec<_> = cfg.pools_fcgi.keys().collect();
    pool_names.sort();
    for name in pool_names {
        let pool = &cfg.pools_fcgi[name];
        out.push_str("[[fcgi_pool]]\n");
        out.push_str(&format!("name = {:?}\n", pool.name));
        out.push_str(&format!("address = {:?}\n", pool.address));
        out.push_str(&format!("transport = {:?}\n", pool.transport));
        if let Some(root) = &pool.document_root {
            out.push_str(&format!("document_root = {:?}\n", root.display()));
        }
        out.push_str(&format!("max_concurrency = {}\n", pool.max_concurrency));
        if let Some(mx) = pool.max_connections {
            out.push_str(&format!("max_connections = {mx}\n"));
        }
        out.push_str(&format!("idle_timeout_ms = {}\n", pool.idle_timeout_ms));
        out.push_str(&format!("total_timeout_ms = {}\n", pool.total_timeout_ms));
        out.push_str(&format!(
            "checkout_timeout_ms = {}\n\n",
            pool.checkout_timeout_ms
        ));
    }

    if !cfg.full_page_cache.enabled {
        out.push_str("[full_page_cache]\nenabled = false\n");
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> std::path::PathBuf {
        let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("exyonq-dump-ir-{nanos}-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn dump_ir_toml_ok() {
        let dir = test_dir();
        let p = dir.join("ok.toml");
        fs::write(
            &p,
            r#"
config_version = 2
[[server]]
listen = "127.0.0.1:18080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
"#,
        )
        .unwrap();
        let r = dump_ir(&p, DumpIrFormat::Toml);
        assert_eq!(r.exit, CliExit::Ok);
        assert!(r.stdout.contains("config_version = 2"));
        assert!(r.stdout.contains("[[server]]"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn dump_ir_json_ok() {
        let dir = test_dir();
        let p = dir.join("ok.toml");
        fs::write(
            &p,
            r#"
config_version = 2
[[server]]
listen = "127.0.0.1:18080"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
"#,
        )
        .unwrap();
        let r = dump_ir(&p, DumpIrFormat::Json);
        assert_eq!(r.exit, CliExit::Ok);
        assert!(r.stdout.contains("\"config_version\""));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn dump_ir_invalid_exits_nonzero() {
        let dir = test_dir();
        let p = dir.join("bad.toml");
        fs::write(&p, "config_version = 2\nnot_a_real_field = 1\n").unwrap();
        let r = dump_ir(&p, DumpIrFormat::Toml);
        assert_eq!(r.exit, CliExit::DiagnosticError);
        let _ = fs::remove_dir_all(dir);
    }
}
