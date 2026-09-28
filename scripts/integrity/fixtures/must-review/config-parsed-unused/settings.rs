// MUST_REVIEW: config field parsed but never consumed.
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Settings {
    pub enable_fancy: bool,
}

// CONFIG_PARSED_BUT_UNUSED: enable_fancy is deserialized then ignored
pub fn apply(_s: &Settings) {
    // no field reads
}
