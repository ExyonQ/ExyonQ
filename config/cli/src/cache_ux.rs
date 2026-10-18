//! Q-04 / ADR-057 — offline `cache explain` (request eligibility).

use crate::output::{sanitize_control_chars, CliExit, OutputFormat, ToolResult};
use crate::reload::load_app_for_reload;
use exyonq_module_api::fpc_request_evaluate;
use std::path::Path;

/// Explain why a URL would (not) enter FPC lookup under the candidate config.
pub fn cache_explain(config_path: &Path, url: &str, format: OutputFormat) -> ToolResult {
    let cfg = match load_app_for_reload(config_path) {
        Ok(c) => c,
        Err(d) => {
            return ToolResult {
                exit: CliExit::DiagnosticError,
                stdout: String::new(),
                stderr: sanitize_control_chars(&d.message),
            };
        }
    };

    let (path, query, host) = match split_http_url(url) {
        Ok(t) => t,
        Err(e) => {
            return ToolResult {
                exit: CliExit::UsageOrTool,
                stdout: String::new(),
                stderr: e,
            };
        }
    };

    let fpc_enabled = cfg.full_page_cache.enabled;
    let eval = fpc_request_evaluate("GET", &path, &query, &[], &[], &[]);

    let (decision, reason, detail_owned): (&str, &str, String) = if !fpc_enabled {
        (
            "skip",
            "disabled",
            "full_page_cache.enabled = false (no X-Cache on this process)".into(),
        )
    } else {
        match &eval {
            Ok((canon_path, canon_query)) => (
                "eligible",
                "none",
                format!(
                    "would MISS then HIT after store; canon_path={canon_path} canon_query={canon_query} host={host}"
                ),
            ),
            Err(r) => (
                "bypass",
                r.as_str(),
                "fpc_request_evaluate rejected — X-Cache: bypass + X-Cache-Reason".into(),
            ),
        }
    };

    match format {
        OutputFormat::Human => ToolResult {
            exit: CliExit::Ok,
            stdout: format!(
                "url = {url}\ndecision = {decision}\nreason = {reason}\ndetail = {detail_owned}\n"
            ),
            stderr: String::new(),
        },
        OutputFormat::Json => ToolResult {
            exit: CliExit::Ok,
            stdout: format!(
                "{{\n  \"schema_version\": 1,\n  \"url\": \"{}\",\n  \"decision\": \"{decision}\",\n  \"reason\": \"{reason}\",\n  \"detail\": \"{}\"\n}}",
                json_escape(url),
                json_escape(&detail_owned)
            ),
            stderr: String::new(),
        },
    }
}

fn split_http_url(url: &str) -> Result<(String, String, String), String> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| "url must start with http:// or https://".to_string())?;
    let (host_part, path_query) = rest
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((rest, "/".into()));
    let host = host_part
        .split_once('@')
        .map(|(_, h)| h)
        .unwrap_or(host_part)
        .split_once(':')
        .map(|(h, _)| h)
        .unwrap_or(host_part)
        .to_string();
    let (path, query) = path_query
        .split_once('?')
        .map(|(p, q)| (p.to_string(), q.to_string()))
        .unwrap_or((path_query, String::new()));
    Ok((path, query, host))
}

fn json_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '"' => "\\\"".to_string(),
            '\\' => "\\\\".to_string(),
            '\n' => "\\n".to_string(),
            c if c.is_control() => " ".to_string(),
            c => c.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn explain_eligible_and_wp_admin_bypass() {
        let dir = std::env::temp_dir().join(format!(
            "exyonq-q04-explain-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("ok.toml");
        fs::write(
            &p,
            r#"
config_version = 1
[[server]]
listen = "127.0.0.1:1"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/t"
[full_page_cache]
enabled = true
namespace = 1
max_entries = 100
max_total_bytes = 1048576
max_object_bytes = 1048576
default_ttl_seconds = 30
max_ttl_seconds = 300
"#,
        )
        .unwrap();
        let r = cache_explain(&p, "http://example.test/", OutputFormat::Human);
        assert_eq!(r.exit, CliExit::Ok, "{}", r.stderr);
        assert!(r.stdout.contains("decision = eligible"), "{}", r.stdout);
        let r2 = cache_explain(&p, "http://example.test/wp-admin/", OutputFormat::Human);
        assert!(r2.stdout.contains("decision = bypass"), "{}", r2.stdout);
        assert!(r2.stdout.contains("reason = wp_admin"), "{}", r2.stdout);
        let _ = fs::remove_dir_all(dir);
    }
}
