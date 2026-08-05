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
//! Path routing: radix tree (matchit) with host-aware fallback.

use exyonq_config_ir::RouteConfig;
use matchit::Router;

/// Radix route index for O(path depth) matching.
#[derive(Clone, Debug)]
pub struct RouteIndex {
    routes: Vec<RouteConfig>,
    router: Router<usize>,
}

impl RouteIndex {
    pub fn new(routes: Vec<RouteConfig>) -> Self {
        let mut router = Router::new();
        // Longest paths first so more specific patterns win when overlapping.
        let mut indexed: Vec<(usize, &RouteConfig)> = routes.iter().enumerate().collect();
        indexed.sort_by_key(|(_, route)| std::cmp::Reverse(route.r#match.path.len()));

        for (idx, route) in &indexed {
            if let Ok(pattern) = path_to_matchit_pattern(&route.r#match.path) {
                let _ = router.insert(&pattern, *idx);
            }
        }

        Self { routes, router }
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
        let path = normalize_path(path);
        if let Ok(idx) = self.router.at(&path) {
            let route_idx = *idx.value;
            let route = &self.routes[route_idx];
            if host_matches_route(route, host) {
                return Some((route_idx, route));
            }
        }
        // Prefix fallback for paths not covered by matchit patterns (trailing segments).
        self.prefix_fallback_index(&path, host)
    }

    pub fn match_route_with_host(&self, path: &str, host: Option<&str>) -> Option<&RouteConfig> {
        self.match_route_index_with_host(path, host)
            .map(|(_, route)| route)
    }

    fn prefix_fallback_index(
        &self,
        path: &str,
        host: Option<&str>,
    ) -> Option<(usize, &RouteConfig)> {
        self.routes
            .iter()
            .enumerate()
            .filter(|(_, route)| {
                path.starts_with(route.r#match.path.trim_end_matches('/'))
                    || (route.r#match.path == "/" && path.starts_with('/'))
            })
            .filter(|(_, route)| host_matches_route(route, host))
            .max_by_key(|(_, route)| route.r#match.path.len())
    }

    /// Exposes the normalized/canonical IR routes for callers that need parallel alignment.
    pub fn routes(&self) -> &[RouteConfig] {
        &self.routes
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

/// Map ExyonQ prefix routes to matchit patterns (`/api` → `/api/{*rest}`).
/// File-like paths (e.g. `routeNNN.bin`) use exact match.
fn path_to_matchit_pattern(path: &str) -> Result<String, matchit::InsertError> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        Ok("/{*rest}".to_string())
    } else if trimmed
        .rsplit('/')
        .next()
        .is_some_and(|segment| segment.contains('.'))
    {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("{trimmed}/{{*rest}}"))
    }
}

fn host_matches_route(route: &RouteConfig, host: Option<&str>) -> bool {
    match (&route.r#match.host, host) {
        (None, _) => true,
        (Some(expected), Some(actual)) => host_matches(expected, actual),
        (Some(_), None) => false,
    }
}

fn host_matches(expected: &str, actual: &str) -> bool {
    expected.eq_ignore_ascii_case(actual)
        || expected
            .strip_prefix('*')
            .is_some_and(|suffix| actual.ends_with(suffix))
}

/// Legacy helper: longest prefix among slice (tests and ad-hoc callers).
pub fn match_route<'a>(routes: &'a [RouteConfig], path: &str) -> Option<&'a RouteConfig> {
    let path = normalize_path(path);
    routes
        .iter()
        .filter(|route| {
            path.starts_with(route.r#match.path.trim_end_matches('/'))
                || (route.r#match.path == "/" && path.starts_with('/'))
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
    fn prefix_fallback_index_matches_longest_prefix() {
        let routes = vec![route("legacy", "/legacy"), route("api", "/api")];
        let index = RouteIndex::new(routes);
        let (idx, matched) = index
            .match_route_index_with_host("/legacy-app/extra", None)
            .expect("prefix fallback");
        assert_eq!(idx, 0);
        assert_eq!(matched.name, "legacy");
    }
}
