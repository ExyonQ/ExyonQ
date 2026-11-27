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
//! F-13: compile-time gate for the Competitive Frontier H1 dataplane.
//!
//! The product default build has the `cfd` feature OFF, so the dataplane
//! launcher and its control crate are not compiled into `exyonq`. An explicit
//! runtime request for the path must fail closed instead of being silently
//! ignored.

pub const ENV_ENABLED: &str = "EXYONQ_COMPETITIVE_H1_DATAPLANE";

fn enabled_value(value: &str) -> bool {
    matches!(value, "1" | "true" | "TRUE" | "yes" | "YES")
}

pub fn requested() -> bool {
    std::env::var(ENV_ENABLED).is_ok_and(|value| enabled_value(&value))
}

pub fn reject_if_requested() -> anyhow::Result<()> {
    if requested() {
        anyhow::bail!(
            "{ENV_ENABLED} is set, but this exyonq binary was built without the `cfd` \
             feature; rebuild with `--features cfd` or unset it"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::enabled_value;

    #[test]
    fn cfd_gate_accepts_only_explicit_truthy_values() {
        for value in ["1", "true", "TRUE", "yes", "YES"] {
            assert!(enabled_value(value), "{value:?} must enable CFD");
        }
        for value in ["", "0", "false", "off", "on", "True", "true "] {
            assert!(!enabled_value(value), "{value:?} must not enable CFD");
        }
    }
}
