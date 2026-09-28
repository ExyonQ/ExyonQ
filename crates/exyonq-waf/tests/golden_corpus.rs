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
//! Golden allow/block corpora for built-in EXY-* detectors (WAF3).

use exyonq_waf::{
    BuiltinDetectors, DetectorToggle, ExclusionInput, IpFilterInput, MapHeaders, NativeWafEngine,
    WafCompileInput,
};
use exyonq_waf_api::{
    EmptyHeaders, HeaderView, WafAction, WafEngine, WafMode, WafPhase, WafRequest,
};
use std::net::{IpAddr, Ipv4Addr};

fn engine_block() -> NativeWafEngine {
    let input = WafCompileInput {
        mode: WafMode::Block,
        ..Default::default()
    };
    // Header anomaly / bots stay Log by default — fine for corpus.
    NativeWafEngine::compile(input).expect("compile builtins")
}

fn req<'a>(
    path: &'a str,
    query: Option<&'a str>,
    headers: &'a dyn HeaderView,
    ip: IpAddr,
) -> WafRequest<'a> {
    WafRequest {
        method: "GET",
        host: Some("example.com"),
        path,
        query,
        headers,
        client_ip: ip,
        route_id: None,
    }
}

fn assert_blocks(path: &str, query: Option<&str>, rule_prefix: &str) {
    let engine = engine_block();
    let headers = MapHeaders::from_pairs(&[("user-agent", "Mozilla/5.0 ExyonQTest")]);
    let d = engine.inspect(
        WafPhase::RequestHeaders,
        &req(path, query, &headers, IpAddr::V4(Ipv4Addr::LOCALHOST)),
    );
    assert_eq!(d.action, WafAction::Block, "path={path} query={query:?}");
    assert!(
        d.violations
            .iter()
            .any(|v| v.rule_id.as_str().starts_with(rule_prefix)),
        "expected {rule_prefix}* in {:?}",
        d.violations
            .iter()
            .map(|v| v.rule_id.as_str())
            .collect::<Vec<_>>()
    );
}

fn assert_allows(path: &str, query: Option<&str>) {
    let engine = engine_block();
    let headers = MapHeaders::from_pairs(&[("user-agent", "Mozilla/5.0 ExyonQTest")]);
    let d = engine.inspect(
        WafPhase::RequestHeaders,
        &req(path, query, &headers, IpAddr::V4(Ipv4Addr::LOCALHOST)),
    );
    assert_eq!(
        d.action,
        WafAction::Allow,
        "unexpected {:?} for path={path} query={query:?}",
        d.violations
    );
}

#[test]
fn golden_block_sqli() {
    assert_blocks(
        "/api",
        Some("id=1 UNION SELECT password FROM users"),
        "EXY-SQL-",
    );
    assert_blocks("/api", Some("x=1 OR 1=1"), "EXY-SQL-");
}

#[test]
fn golden_block_xss() {
    assert_blocks("/search", Some("q=<script>alert(1)</script>"), "EXY-XSS-");
    assert_blocks("/x", Some("a=javascript:alert(1)"), "EXY-XSS-");
}

#[test]
fn golden_block_path_and_encoded() {
    assert_blocks("/static/../../../etc/passwd", None, "EXY-PATH-");
    // Double-encoded ../ within normalize budget
    assert_blocks("/files/%252e%252e%252fetc/passwd", None, "EXY-PATH-");
}

#[test]
fn golden_block_cmd() {
    assert_blocks("/exec", Some("cmd=test;cat /etc/passwd"), "EXY-CMD-");
}

#[test]
fn golden_allow_benign() {
    assert_allows("/api/users/123/profile", None);
    assert_allows("/docs/selecting-widgets", None); // "select" alone should not trip EXY-SQL-1001
    assert_allows("/blog/about-unions", None);
}

#[test]
fn golden_evidence_includes_url_decode_transform() {
    let engine = engine_block();
    let headers = MapHeaders::from_pairs(&[("user-agent", "Mozilla/5.0 ExyonQTest")]);
    let d = engine.inspect(
        WafPhase::RequestHeaders,
        &req(
            "/files/%252e%252e%252fetc/passwd",
            None,
            &headers,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        ),
    );
    assert_eq!(d.action, WafAction::Block);
    let ev = &d.violations[0].evidence;
    assert!(
        ev.transforms
            .iter()
            .any(|t| matches!(t, exyonq_waf_api::WafTransform::UrlDecode { .. })),
        "expected UrlDecode evidence, got {:?}",
        ev.transforms
    );
}

#[test]
fn golden_bot_logs_not_blocks_by_default() {
    let engine = engine_block();
    let headers = MapHeaders::from_pairs(&[("user-agent", "sqlmap/1.7")]);
    let d = engine.inspect(
        WafPhase::RequestHeaders,
        &req("/", None, &headers, IpAddr::V4(Ipv4Addr::LOCALHOST)),
    );
    assert_eq!(d.action, WafAction::Log);
    assert!(d
        .violations
        .iter()
        .any(|v| v.rule_id.as_str().starts_with("EXY-BOT-")));
}

#[test]
fn golden_ip_blacklist() {
    let input = WafCompileInput {
        mode: WafMode::Block,
        builtins: BuiltinDetectors {
            ip_filter: IpFilterInput {
                enabled: true,
                whitelist: vec![],
                blacklist: vec!["10.0.0.0/8".into()],
            },
            // Avoid header anomaly noise
            header_anomaly: DetectorToggle::off(),
            bot_ua: DetectorToggle::off(),
            ..Default::default()
        },
        ..Default::default()
    };
    let engine = NativeWafEngine::compile(input).unwrap();
    let headers = EmptyHeaders;
    let d = engine.inspect(
        WafPhase::RequestHeaders,
        &req("/", None, &headers, IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3))),
    );
    assert_eq!(d.action, WafAction::Block);
    assert_eq!(d.violations[0].rule_id.as_str(), "EXY-IP-1001");
}

#[test]
fn golden_exclusion_suppresses_xss() {
    let input = WafCompileInput {
        mode: WafMode::Block,
        exclusions: vec![ExclusionInput {
            host: Some("example.com".into()),
            path_prefix: Some("/search".into()),
            method: Some("GET".into()),
            rule_ids: vec!["EXY-XSS-1001".into()],
        }],
        ..Default::default()
    };
    let engine = NativeWafEngine::compile(input).unwrap();
    let headers = MapHeaders::from_pairs(&[("user-agent", "Mozilla/5.0 ExyonQTest")]);
    let d = engine.inspect(
        WafPhase::RequestHeaders,
        &req(
            "/search",
            Some("q=<script>alert(1)</script>"),
            &headers,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        ),
    );
    // EXY-XSS-1001 excluded; other XSS rules may still fire — ensure 1001 absent
    assert!(!d
        .violations
        .iter()
        .any(|v| v.rule_id.as_str() == "EXY-XSS-1001"));
    assert_eq!(d.action, WafAction::Allow);
}
