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
//! Q-01 / ADR-052 — predictive (advisory) config lint on validated IR.
//!
//! Pure analysis: no DNS, no health probes, no `core/`.

use exyonq_config_ir::{
    AppConfig, Diagnostic, DiagnosticCode, RouteConfig, RouteMatch, RouteMatchKind,
};

const DOC_SHADED: &str = "docs/config/diagnostic-codes.md#exy-lint-0001";
const DOC_DUP_HDR: &str = "docs/config/diagnostic-codes.md#exy-lint-0002";
const DOC_NO_HC: &str = "docs/config/diagnostic-codes.md#exy-lint-0003";
const DOC_WEAK_RE: &str = "docs/config/diagnostic-codes.md#exy-lint-0004";
const DOC_UNUSED_MAP: &str = "docs/config/diagnostic-codes.md#exy-lint-0005";
const DOC_HOSTLESS: &str = "docs/config/diagnostic-codes.md#exy-lint-0006";
const DOC_FPC_NONCE: &str = "docs/config/diagnostic-codes.md#exy-lint-0007";

/// WordPress nonces are tick-bound: a 12h tick and a 12–24h life. A cached
/// anonymous page keeps its nonce valid only while the serve window stays
/// below one tick (OL-02 alternative; no ESI).
const FPC_NONCE_MAX_WINDOW_SECS: u64 = 43_200;

/// Emit advisory warnings for footguns that still parse as valid IR.
pub fn analyze_predictive(cfg: &AppConfig) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    shaded_routes(cfg, &mut out);
    weak_regex_routes(cfg, &mut out);
    hostless_catchall(cfg, &mut out);
    duplicate_proxy_headers(cfg, &mut out);
    upstreams_without_health(cfg, &mut out);
    unused_maps(cfg, &mut out);
    fpc_nonce_window(cfg, &mut out);
    out
}

fn fpc_nonce_window(cfg: &AppConfig, out: &mut Vec<Diagnostic>) {
    let fpc = &cfg.full_page_cache;
    if !fpc.enabled {
        return;
    }
    let window = fpc.max_ttl_seconds.saturating_add(fpc.stale_if_error_secs);
    if window <= FPC_NONCE_MAX_WINDOW_SECS {
        return;
    }
    out.push(
        Diagnostic::warning(
            DiagnosticCode::LintFpcNonceWindow,
            format!(
                "full_page_cache serve window is {window}s (max_ttl_seconds + stale_if_error_secs)"
            ),
        )
        .with_field("full_page_cache.max_ttl_seconds")
        .with_cause(
            "anonymous WordPress nonces live 12–24h; a longer window can serve a cached nonce after it expires (OL-02: no ESI)",
        )
        .with_suggestion(
            "keep max_ttl_seconds + stale_if_error_secs ≤ 43200 (recommended ≤ 21600)",
        )
        .with_documentation(DOC_FPC_NONCE),
    );
}

fn host_key(m: &RouteMatch) -> String {
    m.host
        .as_deref()
        .map(|h| h.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

fn path_under(longer: &str, shorter: &str) -> bool {
    if longer == shorter {
        return false;
    }
    if shorter == "/" {
        return longer.starts_with('/');
    }
    if !longer.starts_with(shorter) {
        return false;
    }
    longer.len() > shorter.len()
        && (shorter.ends_with('/') || longer.as_bytes().get(shorter.len()) == Some(&b'/'))
}

fn shaded_routes(cfg: &AppConfig, out: &mut Vec<Diagnostic>) {
    let routes = &cfg.routes;
    for i in 0..routes.len() {
        for j in (i + 1)..routes.len() {
            let a = &routes[i];
            let b = &routes[j];
            if host_key(&a.r#match) != host_key(&b.r#match) {
                continue;
            }
            match (a.r#match.kind(), b.r#match.kind()) {
                (Some(RouteMatchKind::Prefix), Some(RouteMatchKind::Prefix)) => {
                    let pa = a.r#match.path.as_str();
                    let pb = b.r#match.path.as_str();
                    if path_under(pb, pa) {
                        push_shaded(out, a, b, pa, pb);
                    } else if path_under(pa, pb) {
                        push_shaded(out, b, a, pb, pa);
                    }
                }
                (Some(RouteMatchKind::Suffix), Some(RouteMatchKind::Suffix)) => {
                    let sa = a.r#match.suffix.as_deref().unwrap_or("");
                    let sb = b.r#match.suffix.as_deref().unwrap_or("");
                    if sa != sb && sb.ends_with(sa) {
                        push_shaded_suffix(out, a, b, sa, sb);
                    } else if sa != sb && sa.ends_with(sb) {
                        push_shaded_suffix(out, b, a, sb, sa);
                    }
                }
                _ => {}
            }
        }
    }
}

fn push_shaded(
    out: &mut Vec<Diagnostic>,
    shaded: &RouteConfig,
    longer: &RouteConfig,
    short_path: &str,
    long_path: &str,
) {
    out.push(
        Diagnostic::warning(
            DiagnosticCode::LintShadedRoute,
            format!(
                "route `{}` prefix `{short_path}` is partially shaded by `{}` (`{long_path}`)",
                shaded.name, longer.name
            ),
        )
        .with_field(format!("route.{}", shaded.name))
        .with_cause(format!(
            "same-host longer prefix wins under `{long_path}` (ADR-047 ranking)"
        ))
        .with_suggestion(format!(
            "narrow `{}` or remove overlap with `{}`; see match ranking contract",
            shaded.name, longer.name
        ))
        .with_documentation(DOC_SHADED),
    );
}

fn push_shaded_suffix(
    out: &mut Vec<Diagnostic>,
    shaded: &RouteConfig,
    longer: &RouteConfig,
    short_sfx: &str,
    long_sfx: &str,
) {
    out.push(
        Diagnostic::warning(
            DiagnosticCode::LintShadedRoute,
            format!(
                "route `{}` suffix `{short_sfx}` is partially shaded by `{}` (`{long_sfx}`)",
                shaded.name, longer.name
            ),
        )
        .with_field(format!("route.{}", shaded.name))
        .with_cause(format!(
            "when a path ends with `{long_sfx}` both suffixes match; longer wins"
        ))
        .with_suggestion(format!(
            "drop `{}` or make suffixes non-overlapping",
            shaded.name
        ))
        .with_documentation(DOC_SHADED),
    );
}

fn weak_regex_routes(cfg: &AppConfig, out: &mut Vec<Diagnostic>) {
    let regexes: Vec<&RouteConfig> = cfg
        .routes
        .iter()
        .filter(|r| r.r#match.kind() == Some(RouteMatchKind::Regex))
        .collect();
    for i in 0..regexes.len() {
        for j in (i + 1)..regexes.len() {
            let a = regexes[i];
            let b = regexes[j];
            if host_key(&a.r#match) != host_key(&b.r#match) {
                continue;
            }
            let la = a.r#match.pattern_len();
            let lb = b.r#match.pattern_len();
            if la == lb {
                continue;
            }
            let (weak, strong) = if la < lb { (a, b) } else { (b, a) };
            out.push(
                Diagnostic::warning(
                    DiagnosticCode::LintWeakRegexRoute,
                    format!(
                        "route `{}` regex is shorter than `{}`; loses when both match",
                        weak.name, strong.name
                    ),
                )
                .with_field(format!("route.{}", weak.name))
                .with_cause(
                    "ADR-047: same host + kind → longer pattern_len wins (not nginx first-match)",
                )
                .with_suggestion(format!(
                    "merge patterns or raise specificity of `{}`",
                    weak.name
                ))
                .with_documentation(DOC_WEAK_RE),
            );
        }
    }
}

fn hostless_catchall(cfg: &AppConfig, out: &mut Vec<Diagnostic>) {
    let has_host_route = cfg
        .routes
        .iter()
        .any(|r| r.r#match.host.as_ref().is_some_and(|h| !h.is_empty()));
    if !has_host_route {
        return;
    }
    for r in &cfg.routes {
        if r.r#match.kind() != Some(RouteMatchKind::Prefix) || r.r#match.path != "/" {
            continue;
        }
        if r.r#match.host.as_ref().is_some_and(|h| !h.is_empty()) {
            continue;
        }
        out.push(
            Diagnostic::warning(
                DiagnosticCode::LintHostlessCatchAll,
                format!(
                    "hostless catch-all route `{}` loses to any host-specific route (host rank)",
                    r.name
                ),
            )
            .with_field(format!("route.{}", r.name))
            .with_cause("Cap033/ADR-047: exact/wildcard host outranks hostless before path kind")
            .with_suggestion(
                "add `match.host` to the catch-all or accept hostless-only fallback semantics",
            )
            .with_documentation(DOC_HOSTLESS),
        );
    }
}

fn duplicate_proxy_headers(cfg: &AppConfig, out: &mut Vec<Diagnostic>) {
    for route in &cfg.routes {
        let mut seen: Vec<(String, usize)> = Vec::new();
        for (idx, h) in route.proxy_set_headers.iter().enumerate() {
            let key = h.name.to_ascii_lowercase();
            if let Some((_, first)) = seen.iter().find(|(k, _)| k == &key) {
                out.push(
                    Diagnostic::warning(
                        DiagnosticCode::LintDuplicateProxyHeader,
                        format!(
                            "route `{}` sets header `{}` more than once (indices {first} and {idx})",
                            route.name, h.name
                        ),
                    )
                    .with_field(format!("route.{}.proxy_set_headers", route.name))
                    .with_cause(
                        "later entries overwrite earlier ones for the same header name",
                    )
                    .with_suggestion(
                        "keep a single `proxy_set_headers` entry per header name",
                    )
                    .with_documentation(DOC_DUP_HDR),
                );
            } else {
                seen.push((key, idx));
            }
        }
    }
}

fn upstreams_without_health(cfg: &AppConfig, out: &mut Vec<Diagnostic>) {
    let mut names: Vec<&str> = cfg.upstreams.keys().map(String::as_str).collect();
    names.sort_unstable();
    for name in names {
        let Some(u) = cfg.upstreams.get(name) else {
            continue;
        };
        if u.endpoint_set.is_empty() {
            continue;
        }
        if u.health_check.enabled {
            continue;
        }
        let n = u.endpoint_set.len();
        out.push(
            Diagnostic::warning(
                DiagnosticCode::LintUpstreamNoHealthCheck,
                format!("upstream `{name}` has {n} endpoint(s) but health_check is disabled"),
            )
            .with_field(format!("upstream.{name}.health_check"))
            .with_cause("failed backends stay selected until connection errors (QUEJAS_NGINX §2.3)")
            .with_suggestion(format!(
                "set `health_check = {{ enabled = true, ... }}` on upstream `{name}`"
            ))
            .with_documentation(DOC_NO_HC),
        );
    }
}

fn unused_maps(cfg: &AppConfig, out: &mut Vec<Diagnostic>) {
    let mut names: Vec<&str> = cfg.maps.keys().map(String::as_str).collect();
    names.sort_unstable();
    for name in names {
        let needle = format!("${name}");
        let used = cfg.routes.iter().any(|r| {
            r.proxy_set_headers
                .iter()
                .any(|h| h.value.contains(needle.as_str()))
        });
        if used {
            continue;
        }
        out.push(
            Diagnostic::warning(
                DiagnosticCode::LintUnusedMap,
                format!("map `{name}` is never referenced from proxy_set_headers"),
            )
            .with_field(format!("map.{name}"))
            .with_cause("map tables only expand via `$name` in header templates (ADR-051)")
            .with_suggestion(format!(
                "reference `${name}` in a route `proxy_set_headers` value or remove the map"
            ))
            .with_documentation(DOC_UNUSED_MAP),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_config_ir::Severity;

    fn parse(toml: &str) -> AppConfig {
        AppConfig::parse_str(toml).expect("fixture must parse")
    }

    #[test]
    fn fixture_emits_at_least_ten_warnings_with_cause_and_suggestion() {
        let cfg = parse(
            r#"
config_version = 1

[[server]]
listen = "127.0.0.1:18080"
routes = ["site", "api", "api_v1", "php", "php_long", "sfx", "sfx_long", "host_a", "dup_hdr"]

[[route]]
name = "site"
match = { path = "/" }
root = "/var/www"

[[route]]
name = "api"
match = { path = "/api" }
upstream = "u1"

[[route]]
name = "api_v1"
match = { path = "/api/v1" }
upstream = "u2"

[[route]]
name = "php"
match = { regex = "\\.php$" }
upstream = "u3"

[[route]]
name = "php_long"
match = { regex = "admin/.*\\.php$" }
upstream = "u3"

[[route]]
name = "sfx"
match = { suffix = ".js" }
root = "/var/www"

[[route]]
name = "sfx_long"
match = { suffix = ".min.js" }
root = "/var/www"

[[route]]
name = "host_a"
match = { path = "/", host = "a.example" }
root = "/var/www/a"

[[route]]
name = "dup_hdr"
match = { path = "/proxy" }
upstream = "u1"
proxy_set_headers = [
  { name = "X-Real-IP", value = "$remote_addr" },
  { name = "x-real-ip", value = "$host" },
]

[[upstream]]
name = "u1"
target = "http://127.0.0.1:9001"

[[upstream]]
name = "u2"
target = "http://127.0.0.1:9002"

[[upstream]]
name = "u3"
target = "http://127.0.0.1:9003"

[[map]]
name = "unused_a"
input = "host"
default = "x"
values = {}

[[map]]
name = "unused_b"
input = "scheme"
default = "http"
values = {}
"#,
        );
        // dup_hdr route must be wired into a server for IR validity — attach via extra server
        // if validate requires all routes referenced; parse_str validates full graph.
        let diags = analyze_predictive(&cfg);
        let warns: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect();
        assert!(
            warns.len() >= 10,
            "expected ≥10 warnings, got {}: {:?}",
            warns.len(),
            warns.iter().map(|d| d.code.as_str()).collect::<Vec<_>>()
        );
        for w in &warns {
            assert!(w.cause.is_some(), "missing cause on {}", w.code);
            assert!(w.suggestion.is_some(), "missing suggestion on {}", w.code);
        }
    }

    #[test]
    fn fpc_nonce_window_warns_only_above_one_tick() {
        fn cfg(ttl: u64, stale: u64, enabled: bool) -> AppConfig {
            parse(&format!(
                r#"
config_version = 1

[[server]]
listen = "127.0.0.1:18080"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/" }}
root = "/var/www"

[full_page_cache]
enabled = {enabled}
max_ttl_seconds = {ttl}
stale_if_error_secs = {stale}
"#
            ))
        }
        let code = DiagnosticCode::LintFpcNonceWindow;

        assert!(analyze_predictive(&cfg(43_200, 0, true))
            .iter()
            .all(|d| d.code != code));
        assert!(analyze_predictive(&cfg(43_200, 1, true))
            .iter()
            .any(|d| d.code == code));
        let warn = analyze_predictive(&cfg(999_999, 0, true))
            .into_iter()
            .find(|d| d.code == code)
            .expect("warning above one tick");
        assert!(warn.cause.is_some() && warn.suggestion.is_some());
        assert!(analyze_predictive(&cfg(999_999, 0, false))
            .iter()
            .all(|d| d.code != code));
    }
}
