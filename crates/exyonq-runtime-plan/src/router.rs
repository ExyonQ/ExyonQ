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
//! Path routing: Cap033 host-aware longest directory-prefix over IR routes.
//!
//! Product matching is linear over `routes` (exact host > wildcard > hostless,
//! then longest path). A matchit radix tree previously lived here but was
//! superseded by Cap033 — matchit cannot rank among vhosts.

use exyonq_config_ir::RouteConfig;

/// Compiled route index for Cap033 host+path selection.
#[derive(Clone, Debug)]
pub struct RouteIndex {
    routes: Vec<RouteConfig>,
}

impl RouteIndex {
    pub fn new(routes: Vec<RouteConfig>) -> Self {
        Self { routes }
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
        // Cap033: rank among all path+host matches (exact host > wildcard > hostless,
        // then longest path). Matchit alone is path-only and cannot choose among vhosts.
        let path = normalize_path(path);
        self.best_host_path_match(&path, host)
    }

    pub fn match_route_with_host(&self, path: &str, host: Option<&str>) -> Option<&RouteConfig> {
        self.match_route_index_with_host(path, host)
            .map(|(_, route)| route)
    }

    /// Longest path among host-matching routes, preferring exact host > wildcard > hostless.
    fn best_host_path_match(
        &self,
        path: &str,
        host: Option<&str>,
    ) -> Option<(usize, &RouteConfig)> {
        self.routes
            .iter()
            .enumerate()
            .filter(|(_, route)| path_under_route_prefix(path, &route.r#match.path))
            .filter(|(_, route)| host_matches_route(route, host))
            .max_by_key(|(_, route)| {
                (
                    host_match_rank(route.r#match.host.as_deref()),
                    route.r#match.path.len(),
                )
            })
    }

    /// Exposes the normalized/canonical IR routes for callers that need parallel alignment.
    pub fn routes(&self) -> &[RouteConfig] {
        &self.routes
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

/// Legacy helper: longest directory-prefix among slice (tests and ad-hoc callers).
pub fn match_route<'a>(routes: &'a [RouteConfig], path: &str) -> Option<&'a RouteConfig> {
    let path = normalize_path(path);
    routes
        .iter()
        .filter(|route| path_under_route_prefix(&path, &route.r#match.path))
        .max_by_key(|route| route.r#match.path.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_config_ir::RouteMatch;

    fn route(name: &str, path: &str) -> RouteConfig {
        RouteConfig {
            name: name.to_string(),
            r#match: RouteMatch {
                path: path.to_string(),
                host: None,
            },
            upstream: Some("backend".to_string()),
            root: None,
            index: None,
            redirect: None,
            rewrite: None,
            fastcgi: None,
            htaccess: Default::default(),
            cache: None,
        }
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
}
