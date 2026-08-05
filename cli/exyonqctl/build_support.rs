//! Shared source-revision resolution for CLI build scripts.
//! Keep free of builder paths and secrets.

/// Forty lowercase hex chars — full git object id.
pub fn is_canonical_git_sha40(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn truthy_env(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

/// Official release builds must not embed `unknown` or empty revisions.
#[allow(dead_code)] // used from build.rs; integration tests call `resolve_source_revision_with`
pub fn official_release_required() -> bool {
    truthy_env("EXYONQ_OFFICIAL_RELEASE") || truthy_env("EXYONQ_REQUIRE_SOURCE_REVISION")
}

/// Core resolver used by build scripts and unit tests (no ambient env reads for flags).
pub fn resolve_source_revision_with(
    forced: Option<&str>,
    git_head: Option<&str>,
    official: bool,
) -> Result<String, String> {
    let forced = forced
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let from_force = forced.is_some();
    let candidate = match forced {
        Some(s) => s,
        None => match git_head.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => s.to_string(),
            None => "unknown".to_string(),
        },
    };

    if official {
        if candidate == "unknown" {
            return Err(
                "EXYONQ_OFFICIAL_RELEASE=1 forbids source_revision=unknown; pass EXYONQ_SOURCE_REVISION=<40-hex>"
                    .into(),
            );
        }
        if !is_canonical_git_sha40(&candidate) {
            return Err(format!(
                "EXYONQ_OFFICIAL_RELEASE=1 requires EXYONQ_SOURCE_REVISION as 40 lowercase hex (got {candidate:?})"
            ));
        }
    } else if from_force && candidate != "unknown" && !is_canonical_git_sha40(&candidate) {
        return Err(format!(
            "EXYONQ_SOURCE_REVISION must be 40 lowercase hex or unset (got {candidate:?})"
        ));
    }

    Ok(candidate)
}

/// Resolve revision: explicit env → optional git → `unknown` (dev only).
#[allow(dead_code)] // used from build.rs; integration tests call `resolve_source_revision_with`
pub fn resolve_source_revision(git_head: Option<String>) -> Result<String, String> {
    let forced = std::env::var("EXYONQ_SOURCE_REVISION").ok();
    resolve_source_revision_with(
        forced.as_deref(),
        git_head.as_deref(),
        official_release_required(),
    )
}
