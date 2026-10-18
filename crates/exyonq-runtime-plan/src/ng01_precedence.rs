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
//! NG-01 precedence suite (≥30 cases). Product ranking is ADR-047 — not nginx-first-regex.
#![cfg(test)]

use crate::router::RouteIndex;
use exyonq_config_ir::{RouteConfig, RouteMatch};

fn r(name: &str, m: RouteMatch) -> RouteConfig {
    RouteConfig {
        name: name.into(),
        r#match: m,
        upstream: Some("u".into()),
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
    }
}

fn hit(index: &RouteIndex, path: &str, host: Option<&str>) -> String {
    index
        .match_route_with_host(path, host)
        .map(|x| x.name.clone())
        .unwrap_or_else(|| "<none>".into())
}

/// Documented suite row: (id, routes, path, host, expected_route).
type PrecedenceCase = (
    &'static str,
    Vec<RouteConfig>,
    &'static str,
    Option<&'static str>,
    &'static str,
);

fn cases() -> Vec<PrecedenceCase> {
    let mut out = Vec::new();

    // 1–5 prefix basics
    out.push((
        "p01_longest_prefix",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("api", RouteMatch::prefix("/api")),
        ],
        "/api/x",
        None,
        "api",
    ));
    out.push((
        "p02_root_fallback",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("api", RouteMatch::prefix("/api")),
        ],
        "/about",
        None,
        "root",
    ));
    out.push((
        "p03_no_false_prefix",
        vec![r("api", RouteMatch::prefix("/api"))],
        "/api2",
        None,
        "<none>",
    ));
    out.push((
        "p04_nested_longer",
        vec![
            r("a", RouteMatch::prefix("/a")),
            r("ab", RouteMatch::prefix("/a/b")),
        ],
        "/a/b/c",
        None,
        "ab",
    ));
    out.push((
        "p05_trailing_slash_dir",
        vec![r("api", RouteMatch::prefix("/api/"))],
        "/api",
        None,
        "api",
    ));

    // 6–12 suffix
    out.push((
        "s01_suffix_over_root",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("php", RouteMatch::with_suffix(".php")),
        ],
        "/i.php",
        None,
        "php",
    ));
    out.push((
        "s02_suffix_miss",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("php", RouteMatch::with_suffix(".php")),
        ],
        "/i.html",
        None,
        "root",
    ));
    out.push((
        "s03_suffix_over_prefix",
        vec![
            r("site", RouteMatch::prefix("/site")),
            r("php", RouteMatch::with_suffix(".php")),
        ],
        "/site/x.php",
        None,
        "php",
    ));
    out.push((
        "s04_longer_suffix",
        vec![
            r("a", RouteMatch::with_suffix(".gz")),
            r("b", RouteMatch::with_suffix(".tar.gz")),
        ],
        "/f.tar.gz",
        None,
        "b",
    ));
    out.push((
        "s05_html",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("html", RouteMatch::with_suffix(".html")),
        ],
        "/x.html",
        None,
        "html",
    ));
    out.push((
        "s06_css",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("css", RouteMatch::with_suffix(".css")),
        ],
        "/a.css",
        None,
        "css",
    ));
    out.push((
        "s07_js",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("js", RouteMatch::with_suffix(".js")),
        ],
        "/a.js",
        None,
        "js",
    ));

    // 13–22 regex
    out.push((
        "x01_regex_over_root",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("ver", RouteMatch::with_regex(r"^/api/v[0-9]+/")),
        ],
        "/api/v2/x",
        None,
        "ver",
    ));
    out.push((
        "x02_prefix_when_no_regex",
        vec![
            r("api", RouteMatch::prefix("/api")),
            r("ver", RouteMatch::with_regex(r"^/api/v[0-9]+/")),
        ],
        "/api/plain",
        None,
        "api",
    ));
    out.push((
        "x03_longer_regex",
        vec![
            r("short", RouteMatch::with_regex(r"^/api/v[0-9]+/")),
            r("long", RouteMatch::with_regex(r"^/api/v[0-9]+/users")),
        ],
        "/api/v2/users/1",
        None,
        "long",
    ));
    out.push((
        "x04_regex_blog",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("blog", RouteMatch::with_regex(r"^/blog/")),
        ],
        "/blog/a",
        None,
        "blog",
    ));
    out.push((
        "x05_regex_digits",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("id", RouteMatch::with_regex(r"^/id/[0-9]+$")),
        ],
        "/id/42",
        None,
        "id",
    ));
    out.push((
        "x06_regex_miss",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("id", RouteMatch::with_regex(r"^/id/[0-9]+$")),
        ],
        "/id/x",
        None,
        "root",
    ));
    out.push((
        "x07_regex_eq_suffix_kind",
        vec![
            r("suf", RouteMatch::with_suffix(".php")),
            r("re", RouteMatch::with_regex(r"\.php$")),
        ],
        "/a.php",
        None,
        "re",
    )); // same kind_rank; longer pattern wins (.php$ len 5 vs .php len 4) — document
    out.push((
        "x08_anchor_end",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("exact", RouteMatch::with_regex(r"^/only$")),
        ],
        "/only",
        None,
        "exact",
    ));
    out.push((
        "x09_anchor_end_miss",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("exact", RouteMatch::with_regex(r"^/only$")),
        ],
        "/only/",
        None,
        "root",
    ));
    out.push((
        "x10_unicode_path_prefix",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("u", RouteMatch::prefix("/ü")),
        ],
        "/ü/x",
        None,
        "u",
    ));

    // 23–30 host
    out.push((
        "h01_exact_over_hostless",
        vec![r("def", RouteMatch::prefix("/")), {
            let mut m = RouteMatch::prefix("/");
            m.host = Some("a.example".into());
            r("a", m)
        }],
        "/",
        Some("a.example"),
        "a",
    ));
    out.push((
        "h02_hostless_other",
        vec![r("def", RouteMatch::prefix("/")), {
            let mut m = RouteMatch::prefix("/");
            m.host = Some("a.example".into());
            r("a", m)
        }],
        "/",
        Some("other.example"),
        "def",
    ));
    out.push((
        "h03_exact_over_wild",
        vec![
            {
                let mut m = RouteMatch::prefix("/");
                m.host = Some("*.example.com".into());
                r("w", m)
            },
            {
                let mut m = RouteMatch::prefix("/");
                m.host = Some("api.example.com".into());
                r("e", m)
            },
        ],
        "/",
        Some("api.example.com"),
        "e",
    ));
    out.push((
        "h04_wild_match",
        vec![{
            let mut m = RouteMatch::prefix("/");
            m.host = Some("*.example.com".into());
            r("w", m)
        }],
        "/",
        Some("x.example.com"),
        "w",
    ));
    out.push((
        "h05_host_beats_suffix",
        vec![r("php", RouteMatch::with_suffix(".php")), {
            let mut m = RouteMatch::prefix("/");
            m.host = Some("app.example".into());
            r("named", m)
        }],
        "/x.php",
        Some("app.example"),
        "named",
    ));
    out.push((
        "h06_suffix_other_host",
        vec![r("php", RouteMatch::with_suffix(".php")), {
            let mut m = RouteMatch::prefix("/");
            m.host = Some("app.example".into());
            r("named", m)
        }],
        "/x.php",
        Some("other.example"),
        "php",
    ));
    out.push((
        "h07_case_normalize",
        vec![{
            let mut m = RouteMatch::prefix("/");
            m.host = Some("Host-A.Example".into());
            r("a", m)
        }],
        "/",
        Some("host-a.example."),
        "a",
    ));
    out.push((
        "h08_host_regex",
        vec![r("any", RouteMatch::with_regex(r"^/x")), {
            let mut m = RouteMatch::with_regex(r"^/x");
            m.host = Some("a.example".into());
            r("ha", m)
        }],
        "/x",
        Some("a.example"),
        "ha",
    ));

    // 31–34 ties / catch-all
    out.push((
        "t01_lower_index_tie_break_doc",
        vec![
            r("first", RouteMatch::with_regex(r"^/z/")),
            r("second", RouteMatch::with_regex(r"^/z/")), // duplicate pattern — plan would fail; runtime last-wins Reverse keeps first
        ],
        "/z/1",
        None,
        "first",
    ));
    out.push((
        "t02_catchall_lowest",
        vec![
            r("root", RouteMatch::prefix("/")),
            r("api", RouteMatch::prefix("/api")),
        ],
        "/api",
        None,
        "api",
    ));
    out.push((
        "t03_wp_includes",
        vec![
            r("health", RouteMatch::prefix("/health")),
            r("inc", RouteMatch::prefix("/wp-includes")),
            r("wp", RouteMatch::prefix("/")),
        ],
        "/wp-includes/css/a.css",
        None,
        "inc",
    ));
    out.push((
        "t04_file_like",
        vec![
            r("site", RouteMatch::prefix("/site")),
            r("bin", RouteMatch::prefix("/site/routes/route050.bin")),
        ],
        "/site/routes/route050.bin",
        None,
        "bin",
    ));

    out
}

#[test]
fn ng01_precedence_suite_at_least_30() {
    let suite = cases();
    assert!(
        suite.len() >= 30,
        "NG-01 DoD requires ≥30 cases, got {}",
        suite.len()
    );
    let mut failed = Vec::new();
    for (id, routes, path, host, expect) in suite {
        let index = RouteIndex::new(routes);
        let got = hit(&index, path, host);
        if got != expect {
            failed.push(format!(
                "{id}: path={path} host={host:?} expected={expect} got={got}"
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "NG-01 precedence failures:\n{}",
        failed.join("\n")
    );
}
