//! Development probe: report the OWASP CRS 4.25 parser accounting.
//!
//! Run with:
//!   EXYONQ_CRS_DIR=$PWD/target/crs-v4.25.0 \
//!   cargo test -p exyonq-waf --test crs_scope_audit -- --ignored --nocapture
//!
//! The differential gate in `scripts/gates/crs-diff-gate.sh` is authoritative;
//! this probe exists so the "malformed vs out-of-scope" split can be read from
//! the real corpus instead of a fixture. It loads the checkout exactly like the
//! CLI does: a main file that `Include`s `crs-setup.conf.example` and every
//! `rules/*.conf`, parsed relative to the main file's directory.

use exyonq_waf::parse_rule_file_with_base;
use std::path::{Path, PathBuf};

fn load_ruleset(dir: &Path) -> exyonq_waf::CrsRulesetInput {
    let work = std::env::temp_dir().join(format!("exq-crs-scope-audit-{}", std::process::id()));
    std::fs::create_dir_all(&work).expect("create audit work dir");
    let main = work.join("exy-main.conf");
    std::fs::write(
        &main,
        format!(
            "Include {}/crs-setup.conf.example\nInclude {}/*.conf\n",
            dir.display(),
            dir.join("rules").display()
        ),
    )
    .expect("write main conf");
    let contents = std::fs::read_to_string(&main).expect("read main conf");
    let ruleset = parse_rule_file_with_base("diff", &contents, 5, Some(&work));
    let _ = std::fs::remove_dir_all(&work);
    ruleset
}

#[test]
#[ignore = "requires EXYONQ_CRS_DIR with the real CRS checkout"]
fn crs_scope_accounting() {
    let Ok(dir) = std::env::var("EXYONQ_CRS_DIR") else {
        eprintln!("EXYONQ_CRS_DIR not set; nothing to audit");
        return;
    };
    let dir = PathBuf::from(dir);
    assert!(
        dir.join("crs-setup.conf.example").exists(),
        "not a CRS checkout: {}",
        dir.display()
    );
    let ruleset = load_ruleset(&dir);
    println!(
        "parsed={} malformed={} unsupported={} ignored_actions={}",
        ruleset.rules.len(),
        ruleset.discarded,
        ruleset.unsupported,
        ruleset.ignored_actions.len()
    );
    println!("malformed_diagnostics={}", ruleset.diagnostics.len());
    for diagnostic in &ruleset.diagnostics {
        println!("  MALFORMED {diagnostic}");
    }
    println!(
        "unsupported_diagnostics={}",
        ruleset.unsupported_diagnostics.len()
    );
    for diagnostic in &ruleset.unsupported_diagnostics {
        println!("  UNSUPPORTED {diagnostic}");
    }
    for (name, occurrences) in &ruleset.ignored_actions {
        println!("  IGNORED_ACTION {name} x{occurrences}");
    }
    // A valid upstream ruleset must load under the product default; the
    // differential gate is the end-to-end check, this asserts the compile path.
    let engine = exyonq_waf::FixedWafEngine::compile_with_crs(
        exyonq_waf::WafCompileInput {
            fail_policy: exyonq_waf::FailPolicy::ClosedForInvalidRules,
            ..Default::default()
        },
        std::slice::from_ref(&ruleset),
    )
    .expect("full OWASP CRS 4.25 must compile under closed_for_invalid_rules");
    let snapshot = engine.snapshot();
    assert_eq!(snapshot.compile_stats().discarded, 0);
    assert_eq!(snapshot.compile_stats().unsupported, 0);
    for id in [
        "901320", "901410", "930120", "941100", "942100", "942410", "942431",
    ] {
        assert!(
            ruleset.rules.iter().any(|rule| rule.id == id),
            "CRS rule {id} did not load"
        );
    }
}
