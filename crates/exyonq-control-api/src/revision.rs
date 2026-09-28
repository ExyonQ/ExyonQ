use crate::error::CtrlError;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevisionState {
    pub revision: u64,
}

impl RevisionState {
    pub fn etag(self) -> String {
        format!("W/\"rev-{}\"", self.revision)
    }
}

pub fn state_path(config_path: &Path) -> PathBuf {
    let mut name: OsString = config_path.as_os_str().to_owned();
    name.push(".control-api-state.json");
    PathBuf::from(name)
}

pub fn load(config_path: &Path) -> Result<RevisionState, CtrlError> {
    let path = state_path(config_path);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(RevisionState { revision: 0 }),
        Err(err) => Err(err.into()),
    }
}

pub fn persist(config_path: &Path, state: RevisionState) -> Result<(), CtrlError> {
    let path = state_path(config_path);
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(&state)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

pub fn bump(config_path: &Path) -> Result<RevisionState, CtrlError> {
    let mut state = load(config_path)?;
    state.revision = state.revision.saturating_add(1);
    persist(config_path, state)?;
    Ok(state)
}

pub fn parse_etag(value: &str) -> Option<u64> {
    let inner = value.strip_prefix("W/\"rev-")?.strip_suffix('"')?;
    inner.parse::<u64>().ok()
}

pub fn matches_current(if_match: Option<&str>, current: RevisionState, required: bool) -> bool {
    match if_match {
        None => !required,
        // Present but unparsable/stale → precondition failed (never treat as absent).
        Some(raw) => parse_etag(raw).is_some_and(|rev| rev == current.revision),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etag_parse_accepts_expected_weak_format() {
        assert_eq!(parse_etag("W/\"rev-42\""), Some(42));
        assert_eq!(parse_etag("\"rev-42\""), None);
        assert_eq!(parse_etag("W/\"rev-x\""), None);
    }

    #[test]
    fn required_if_match_rejects_missing_or_stale() {
        let state = RevisionState { revision: 7 };
        assert!(!matches_current(None, state, true));
        assert!(!matches_current(Some("W/\"rev-6\""), state, true));
        assert!(matches_current(Some("W/\"rev-7\""), state, true));
    }

    #[test]
    fn optional_if_match_rejects_malformed_present_header() {
        let state = RevisionState { revision: 7 };
        assert!(matches_current(None, state, false));
        assert!(!matches_current(Some("garbage"), state, false));
        assert!(!matches_current(Some("\"rev-7\""), state, false));
        assert!(matches_current(Some("W/\"rev-7\""), state, false));
    }
}
