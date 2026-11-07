//! Development probe against a real CRS checkout.
//!
//! Run with:
//!   EXYONQ_CRS_DIR=$PWD/.exyonq-local/refs/crs-v4.25.0 \
//!   cargo test -p exyonq-waf --test crs_real_probe -- --ignored --nocapture
//!
//! The differential gate in `scripts/gates/crs-diff.py` is authoritative; this
//! test only exposes which rules scored a decision.

use exyonq_waf::{parse_rule_file_with_base, FixedWafEngine, MapHeaders, WafCompileInput};
use exyonq_waf_api::{BodyInspectionState, WafDecision, WafEngine, WafMode, WafPhase, WafRequest};
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;

fn load_engine() -> Option<FixedWafEngine> {
    let dir = std::env::var("EXYONQ_CRS_DIR").ok()?;
    let dir = Path::new(&dir);
    if !dir.join("crs-setup.conf.example").exists() {
        return None;
    }
    let mut contents = std::fs::read_to_string(dir.join("crs-setup.conf.example")).ok()?;
    let mut rules: Vec<_> = std::fs::read_dir(dir.join("rules"))
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "conf"))
        .collect();
    rules.sort();
    for rule in rules {
        contents.push_str(&std::fs::read_to_string(rule).ok()?);
        contents.push('\n');
    }
    let ruleset = parse_rule_file_with_base("probe", &contents, 5, Some(dir));
    FixedWafEngine::compile_with_crs(
        WafCompileInput {
            mode: WafMode::Block,
            ..Default::default()
        },
        &[ruleset],
    )
    .ok()
}

fn ids(decision: &WafDecision) -> Vec<String> {
    decision
        .violations
        .iter()
        .map(|violation| violation.rule_id.as_str().to_string())
        .collect()
}

fn run_body(
    engine: &FixedWafEngine,
    name: &str,
    method: &str,
    path: &str,
    body: &[u8],
    pairs: &[(&str, &str)],
) {
    let headers = MapHeaders::from_pairs(pairs);
    let (path, query) = path
        .split_once('?')
        .map_or((path, None), |(p, q)| (p, Some(q)));
    let req = WafRequest {
        method,
        host: None,
        path,
        query,
        headers: &headers,
        client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
        route_id: None,
    };
    let mut state = BodyInspectionState::default();
    let decision = engine.inspect_body_chunk(&req, body, true, &mut state);
    println!("{name}: body={:?} {:?}", decision.action, ids(&decision));
}

fn run(engine: &FixedWafEngine, name: &str, method: &str, path: &str, pairs: &[(&str, &str)]) {
    let headers = MapHeaders::from_pairs(pairs);
    let (path, query) = path
        .split_once('?')
        .map_or((path, None), |(p, q)| (p, Some(q)));
    let req = WafRequest {
        method,
        host: None,
        path,
        query,
        headers: &headers,
        client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
        route_id: None,
    };
    let headers_decision = engine.inspect(WafPhase::RequestHeaders, &req);
    let mut state = BodyInspectionState::default();
    let body_decision = engine.inspect_body_chunk(&req, b"", true, &mut state);
    println!(
        "{name}: headers={:?} {:?} | body={:?} {:?}",
        headers_decision.action,
        ids(&headers_decision),
        body_decision.action,
        ids(&body_decision)
    );
}

#[test]
#[ignore = "requires EXYONQ_CRS_DIR with the real CRS checkout"]
fn probe_over_block_cases() {
    let Some(engine) = load_engine() else {
        eprintln!("EXYONQ_CRS_DIR not set or CRS checkout missing");
        return;
    };
    run(
        &engine,
        "920660-t18 Range only",
        "GET",
        "/video.mp4",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
            ("Accept", "*/*"),
            ("Range", "bytes=0-1023"),
        ],
    );
    run(
        &engine,
        "943110-t30 same-domain referer",
        "GET",
        "/login?sessionid=abc123",
        &[
            ("Host", "myapp.com"),
            ("Referer", "https://myapp.com/previous-page"),
            ("User-Agent", "OWASP CRS test agent"),
        ],
    );
    run(
        &engine,
        "920450 restricted Content-range",
        "GET",
        "/",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
            ("Content-range", "test"),
        ],
    );
    run(
        &engine,
        "942500-t3 optimizer hint",
        "GET",
        "/get?test=9999+or+%2F*%2Boptimizer+hint+*%2F+true",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
        ],
    );
    run(
        &engine,
        "942390-t1 sqli",
        "GET",
        "/get?var=sdfsd%27or%201%20%3e%201",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
        ],
    );
    run(
        &engine,
        "942340-t2 sqlite",
        "GET",
        "/get?var=except%20%09select.1%2C2",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
        ],
    );
    run(
        &engine,
        "934160-t4 while falsy",
        "GET",
        "/get?eval=while(!%2B0);",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
        ],
    );
    run(
        &engine,
        "941180-t7 filename fp",
        "GET",
        "/get/javascript-manual/document.cookie",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
        ],
    );
    run(
        &engine,
        "933110-t20 X.Filename php",
        "GET",
        "/upload2",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
            ("X.Filename", "a.php"),
        ],
    );
    run(
        &engine,
        "920460-t2 backslash query",
        "GET",
        "/?file=cat+/etc/pa%5Cssw%5Cd",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
        ],
    );
    run(
        &engine,
        "920610-t2 raw fragment",
        "GET",
        "/#fragment",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
        ],
    );
    run(
        &engine,
        "932200-t12 backslash command",
        "GET",
        "/get?host=www.google.com;cat%20%2Fetc%2Fp%5Casswd",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
        ],
    );
    run(
        &engine,
        "942310-t2 select min",
        "GET",
        "/get?var=%27%20and%201%20in%20(select%20min(name)%20from%20sysobjects)%20--",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
        ],
    );
    run(
        &engine,
        "920100-t8 colon path",
        "GET",
        "/index.html:80?I=Like&Apples=Today#tag",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
        ],
    );
    run(
        &engine,
        "933100-t33 html entity",
        "GET",
        "/get?foo=%26lt%3B%3F%20should%20not%20trigger",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
        ],
    );
    run_body(
        &engine,
        "920240-t4 url encoding body",
        "POST",
        "/",
        b"<SOAP-ENV:Envelope xmlns:SOAP-ENV=\\\"http://schemas.xmlsoap.org/soap/envelope/\\\"><xkms:RespondWith>%1Gwww.attack.org</xkms:RespondWith></SOAP-ENV:Envelope>",
        &[
            ("Host", "localhost"),
            ("Content-Type", "text/xml"),
        ],
    );
    run_body(
        &engine,
        "944300-t1 java body",
        "POST",
        "/post",
        b"test=cnVudGltZQ",
        &[
            ("Host", "localhost"),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
    );
    run(
        &engine,
        "933160-t16 php c comment",
        "GET",
        "/get?foo=exec%09%2f%2afoo%0d%0abar%2a%2f%09%20%28%29",
        &[
            ("Host", "localhost"),
            ("User-Agent", "OWASP CRS test agent"),
        ],
    );
}
