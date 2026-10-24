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
//! Path routing: Cap033 host-aware ranking + ADR-047 regex/suffix (NG-01).
//!
//! Product matching is linear over `routes`:
//! host (exact > wildcard > hostless) → kind (regex/suffix > prefix > catch-all `/`)
//! → longer pattern → lower original index on ties.
//! Regex is compiled once in [`RouteIndex::try_new`], never on the request path.

use std::cmp::Reverse;

use exyonq_config_ir::{RouteConfig, RouteMatchKind};
use regex::Regex;

/// Compiled route index for Cap033 + NG-01 selection.
#[derive(Clone, Debug)]
pub struct RouteIndex {
    routes: Vec<RouteConfig>,
    matchers: Vec<CompiledMatcher>,
}

#[derive(Clone, Debug)]
enum CompiledMatcher {
    Prefix,
    Suffix,
    Regex(Regex),
}

#[derive(Debug)]
pub enum RouteIndexError {
    InvalidKind { route: String },
    RegexCompile { route: String, detail: String },
}

impl std::fmt::Display for RouteIndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKind { route } => write!(
                f,
                "route `{route}` has invalid match kind (need exactly one of path/suffix/regex)"
            ),
            Self::RegexCompile { route, detail } => {
                write!(f, "route `{route}` regex compile failed: {detail}")
            }
        }
    }
}

impl std::error::Error for RouteIndexError {}

impl RouteIndex {
    /// Build index, compiling regex arms once. Prefer this over [`Self::new`].
    pub fn try_new(routes: Vec<RouteConfig>) -> Result<Self, RouteIndexError> {
        let mut matchers = Vec::with_capacity(routes.len());
        for route in &routes {
            let matcher = match route.r#match.kind() {
                Some(RouteMatchKind::Prefix) => CompiledMatcher::Prefix,
                Some(RouteMatchKind::Suffix) => CompiledMatcher::Suffix,
                Some(RouteMatchKind::Regex) => {
                    let pattern = route.r#match.regex.as_deref().unwrap_or("");
                    let re = Regex::new(pattern).map_err(|e| RouteIndexError::RegexCompile {
                        route: route.name.clone(),
                        detail: e.to_string(),
                    })?;
                    CompiledMatcher::Regex(re)
                }
                None => {
                    return Err(RouteIndexError::InvalidKind {
                        route: route.name.clone(),
                    });
                }
            };
            matchers.push(matcher);
        }
        Ok(Self { routes, matchers })
    }

    /// Panic on invalid match (tests with prefix-only fixtures).
    pub fn new(routes: Vec<RouteConfig>) -> Self {
        Self::try_new(routes).expect("RouteIndex::new: invalid route match")
    }

    pub fn match_route(&self, path: &str) -> Option<&RouteConfig> {
        self.match_route_with_host(path, None)
    }

    /// Match path/host and return the route's index in the original IR `routes` vec.
    pub fn match_route_index_with_host(
        &self,
        path: &str,
        host: Option<&str>,
    ) -> Option<(usize, &RouteConfig)> {
        self.match_route_index_with_host_and_headers(path, host, &[])
    }

    /// NG-05 / ADR-051: path/host match plus optional `match.header` AND filter.
    pub fn match_route_index_with_host_and_headers(
        &self,
        path: &str,
        host: Option<&str>,
        request_headers: &[(String, String)],
    ) -> Option<(usize, &RouteConfig)> {
        let path = normalize_path(path);
        self.best_match(&path, host, request_headers)
    }

    pub fn match_route_with_host(&self, path: &str, host: Option<&str>) -> Option<&RouteConfig> {
        self.match_route_index_with_host(path, host)
            .map(|(_, route)| route)
    }

    fn best_match(
        &self,
        path: &str,
        host: Option<&str>,
        request_headers: &[(String, String)],
    ) -> Option<(usize, &RouteConfig)> {
        self.routes
            .iter()
            .zip(self.matchers.iter())
            .enumerate()
            .filter(|(_, (route, matcher))| {
                host_matches_route(route, host)
                    && path_matches(path, route, matcher)
                    && exyonq_config_ir::match_header_satisfied(&route.r#match, request_headers)
            })
            .max_by_key(|(idx, (route, _))| {
                (
                    host_match_rank(route.r#match.host.as_deref()),
                    route.r#match.kind_rank(),
                    route.r#match.pattern_len(),
                    Reverse(*idx),
                )
            })
            .map(|(idx, (route, _))| (idx, route))
    }

    /// Exposes the normalized/canonical IR routes for callers that need parallel alignment.
    pub fn routes(&self) -> &[RouteConfig] {
        &self.routes
    }
}

fn path_matches(path: &str, route: &RouteConfig, matcher: &CompiledMatcher) -> bool {
    match matcher {
        CompiledMatcher::Prefix => path_under_route_prefix(path, &route.r#match.path),
        CompiledMatcher::Suffix => {
            let suffix = route.r#match.suffix.as_deref().unwrap_or("");
            !suffix.is_empty() && path.ends_with(suffix)
        }
        CompiledMatcher::Regex(re) => re.is_match(path),
    }
}

fn host_match_rank(configured: Option<&str>) -> u8 {
    match configured {
        Some(h) if h.starts_with('*') => 1,
        Some(_) => 2,
        None => 0,
    }
}

fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return "/".to_string();
    }
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

/// Directory-prefix semantics for `match.path`:
/// `/api/` matches `/api`, `/api/`, and `/api/...` — but not `/api2`.
fn path_under_route_prefix(path: &str, route_path: &str) -> bool {
    let prefix = route_path.trim_end_matches('/');
    if prefix.is_empty() {
        return path.starts_with('/');
    }
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

/// Map ExyonQ prefix routes to matchit patterns.
///
/// Insert exact prefix forms **and** `{*rest}` so nested route roots like
/// `/api/v2/` match themselves, not a shorter `/api/{*rest}` catch-all.
/// File-like paths (e.g. `routeNNN.bin`) use exact match only.
///
/// Retained for the matchit 0.9 compile-health test only — Cap033 product
/// matching does not use matchit.
#[cfg(test)]
fn path_to_matchit_patterns(path: &str) -> Vec<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return vec!["/".to_string(), "/{*rest}".to_string()];
    }
    if trimmed
        .rsplit('/')
        .next()
        .is_some_and(|segment| segment.contains('.'))
    {
        return vec![trimmed.to_string()];
    }
    vec![
        trimmed.to_string(),
        format!("{trimmed}/"),
        format!("{trimmed}/{{*rest}}"),
    ]
}

fn host_matches_route(route: &RouteConfig, host: Option<&str>) -> bool {
    match (&route.r#match.host, host) {
        (None, _) => true,
        (Some(expected), Some(actual)) => host_matches(expected, actual),
        (Some(_), None) => false,
    }
}

/// Cap033: DNS host labels are case-insensitive; trailing FQDN dots are equivalent.
pub fn normalize_route_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

fn host_matches(expected: &str, actual: &str) -> bool {
    let actual_n = normalize_route_host(actual);
    if let Some(suffix) = expected.strip_prefix('*') {
        // `*.example.com` → suffix `.example.com` (normalized).
        let suffix_n = normalize_route_host(suffix);
        return actual_n.ends_with(&suffix_n);
    }
    normalize_route_host(expected) == actual_n
}

/// Legacy helper: longest directory-prefix among slice (prefix arms only).
pub fn match_route<'a>(routes: &'a [RouteConfig], path: &str) -> Option<&'a RouteConfig> {
    let path = normalize_path(path);
    routes
        .iter()
        .filter(|route| {
            route.r#match.kind() == Some(RouteMatchKind::Prefix)
                && path_under_route_prefix(&path, &route.r#match.path)
        })
        .max_by_key(|route| route.r#match.path.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_config_ir::RouteMatch;

    fn route(name: &str, path: &str) -> RouteConfig {
        RouteConfig {
            name: name.to_string(),
            r#match: RouteMatch::prefix(path),
            upstream: Some("backend".to_string()),
            root: None,
            index: None,
            redirect: None,
            rewrite: None,
            fastcgi: None,
            htaccess: Default::default(),
            cache: None,
            fallback_rewrite: None,
            try_uri: None,
            try_uri_dir: None,
            proxy_set_headers: Vec::new(),
            cache_headers: None,
            response_headers: Vec::new(),
            location_rewrite: Vec::new(),
        }
    }

    fn route_suffix(name: &str, suffix: &str) -> RouteConfig {
        let mut r = route(name, "/");
        r.r#match = RouteMatch::with_suffix(suffix);
        r
    }

    fn route_regex(name: &str, regex: &str) -> RouteConfig {
        let mut r = route(name, "/");
        r.r#match = RouteMatch::with_regex(regex);
        r
    }

    #[test]
    fn matchit_0_9_inserts_nested_prefix_catchalls_without_conflict() {
        // Cap033 product matching is host-aware linear; this test only guards that
        // ExyonQ prefix→matchit pattern mapping still inserts under matchit 0.9
        // conflict rules (dependency retained for this compile-health oracle).
        use matchit::Router;
        let routes = [
            route("root", "/"),
            route("api", "/api"),
            route("api_v2", "/api/v2"),
            route("site", "/site"),
            route("file", "/site/routes/route050.bin"),
        ];
        let mut router = Router::new();
        for (idx, route) in routes.iter().enumerate() {
            for pattern in path_to_matchit_patterns(&route.r#match.path) {
                router
                    .insert(&pattern, idx)
                    .unwrap_or_else(|e| panic!("insert {pattern:?} failed under matchit 0.9: {e}"));
            }
        }
        let hit = router.at("/api/v2/users").expect("api_v2 catch-all");
        assert_eq!(*hit.value, 2);
    }

    #[test]
    fn picks_longest_prefix() {
        let routes = vec![route("root", "/"), route("api", "/api")];
        let index = RouteIndex::new(routes);
        let matched = index.match_route("/api/users").unwrap();
        assert_eq!(matched.name, "api");
    }

    #[test]
    fn matches_many_routes_p7_style() {
        let mut routes = vec![route("site", "/site")];
        for i in 0..100 {
            routes.push(route(
                &format!("route{i:03}"),
                &format!("/site/routes/route{i:03}.bin"),
            ));
        }
        let index = RouteIndex::new(routes);
        let matched = index.match_route("/site/routes/route050.bin").unwrap();
        assert_eq!(matched.name, "route050");
    }

    #[test]
    fn match_route_index_with_host_returns_original_route_index() {
        let routes = vec![route("root", "/"), route("api", "/api")];
        let index = RouteIndex::new(routes);
        let (idx, matched) = index
            .match_route_index_with_host("/api/users", None)
            .expect("api route");
        assert_eq!(idx, 1);
        assert_eq!(matched.name, "api");
        assert_eq!(
            index
                .match_route_with_host("/api/users", None)
                .map(|r| r.name.as_str()),
            Some("api")
        );
    }

    #[test]
    fn wp_includes_static_wins_over_root_catchall() {
        let routes = vec![
            route("health", "/health"),
            route("wp-includes", "/wp-includes"),
            route("wordpress", "/"),
        ];
        let index = RouteIndex::new(routes);
        let (idx, matched) = index
            .match_route_index_with_host("/wp-includes/css/dashicons.min.css", None)
            .expect("wp-includes route");
        assert_eq!(matched.name, "wp-includes");
        assert_eq!(idx, 1);
    }

    #[test]
    fn directory_prefix_does_not_match_false_prefix() {
        let routes = vec![route("legacy", "/legacy"), route("api", "/api")];
        let index = RouteIndex::new(routes);
        // `/legacy` must not select `/legacy-app/...` (string-prefix false match).
        assert!(index
            .match_route_index_with_host("/legacy-app/extra", None)
            .is_none());
        assert!(index.match_route_index_with_host("/api2", None).is_none());
        let matched = index.match_route("/legacy/extra").unwrap();
        assert_eq!(matched.name, "legacy");
    }

    fn route_host(name: &str, path: &str, host: Option<&str>) -> RouteConfig {
        let mut r = route(name, path);
        r.r#match.host = host.map(str::to_string);
        r
    }

    #[test]
    fn hostless_same_path_does_not_shadow_named_host() {
        let routes = vec![
            route_host("default", "/", None),
            route_host("a", "/", Some("host-a.example")),
        ];
        let index = RouteIndex::new(routes);
        let matched = index
            .match_route_with_host("/", Some("host-a.example"))
            .expect("named host");
        assert_eq!(matched.name, "a");
        let def = index
            .match_route_with_host("/", Some("other.example"))
            .expect("hostless default");
        assert_eq!(def.name, "default");
    }

    #[test]
    fn exact_host_beats_overlapping_wildcard() {
        let routes = vec![
            route_host("wild", "/", Some("*.example.com")),
            route_host("exact", "/", Some("api.example.com")),
        ];
        let index = RouteIndex::new(routes);
        assert_eq!(
            index
                .match_route_with_host("/", Some("api.example.com"))
                .unwrap()
                .name,
            "exact"
        );
        assert_eq!(
            index
                .match_route_with_host("/", Some("other.example.com"))
                .unwrap()
                .name,
            "wild"
        );
    }

    #[test]
    fn trailing_dot_and_case_normalize() {
        let routes = vec![route_host("a", "/", Some("host-a.example"))];
        let index = RouteIndex::new(routes);
        assert_eq!(
            index
                .match_route_with_host("/", Some("Host-A.Example."))
                .unwrap()
                .name,
            "a"
        );
    }

    #[test]
    fn suffix_beats_prefix_catchall() {
        let routes = vec![route("root", "/"), route_suffix("php", ".php")];
        let index = RouteIndex::new(routes);
        assert_eq!(index.match_route("/index.php").unwrap().name, "php");
        assert_eq!(index.match_route("/about").unwrap().name, "root");
    }

    #[test]
    fn regex_beats_prefix_and_longer_regex_wins() {
        let routes = vec![
            route("root", "/"),
            route("api", "/api"),
            route_regex("api_ver", r"^/api/v[0-9]+/"),
            route_regex("api_ver_long", r"^/api/v[0-9]+/users"),
        ];
        let index = RouteIndex::new(routes);
        assert_eq!(
            index.match_route("/api/v2/users/1").unwrap().name,
            "api_ver_long"
        );
        assert_eq!(index.match_route("/api/v2/x").unwrap().name, "api_ver");
        assert_eq!(index.match_route("/api/plain").unwrap().name, "api");
    }

    #[test]
    fn exact_host_beats_hostless_regex() {
        let mut named = route("named-prefix", "/");
        named.r#match.host = Some("app.example".into());
        let routes = vec![route_regex("any-php", r"\.php$"), named];
        let index = RouteIndex::new(routes);
        assert_eq!(
            index
                .match_route_with_host("/x.php", Some("app.example"))
                .unwrap()
                .name,
            "named-prefix"
        );
        assert_eq!(
            index
                .match_route_with_host("/x.php", Some("other.example"))
                .unwrap()
                .name,
            "any-php"
        );
    }

    #[test]
    fn tie_prefers_lower_route_index() {
        // Same kind_rank + same pattern_len → lower index wins (ADR-047).
        let routes = vec![
            route_regex("first", r"^/x/"),
            route_regex("second", r"^/y/"), // different pattern; not a tie
        ];
        let index = RouteIndex::new(routes);
        assert_eq!(index.match_route("/x/1").unwrap().name, "first");

        let routes = vec![
            route_suffix("a", ".html"),
            route_suffix("b", ".htm"), // different
        ];
        let index = RouteIndex::new(routes);
        assert_eq!(index.match_route("/p.html").unwrap().name, "a");
    }
}
