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
//! Forwarding header helpers (pure data — no I/O).

/// Headers to append when forwarding to upstream (caller applies to request builder).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ForwardingHeaders {
    pub x_forwarded_for: Option<String>,
    pub host: Option<String>,
}

impl ForwardingHeaders {
    pub fn new(x_forwarded_for: Option<&str>, host: Option<&str>) -> Self {
        Self {
            x_forwarded_for: x_forwarded_for.map(str::to_string),
            host: host.map(str::to_string),
        }
    }

    pub fn as_header_pairs(&self) -> Vec<(String, String)> {
        let mut out = Vec::with_capacity(2);
        if let Some(xff) = &self.x_forwarded_for {
            out.push(("x-forwarded-for".into(), xff.clone()));
        }
        if let Some(host) = &self.host {
            out.push(("host".into(), host.clone()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_xff_and_host() {
        let fh = ForwardingHeaders::new(Some("10.0.0.1"), Some("upstream.local"));
        assert_eq!(
            fh.as_header_pairs(),
            vec![
                ("x-forwarded-for".into(), "10.0.0.1".into()),
                ("host".into(), "upstream.local".into()),
            ]
        );
    }
}
