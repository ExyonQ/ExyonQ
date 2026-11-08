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
//! Upstream URI construction (no Hyper client).
//!
//! L-02: every request path is built from the compiled upstream base. There is
//! no benchmark-path preselection table.

use http::Uri;

/// Build upstream request URI from configured base and client path/query.
pub fn build_uri(base: &Uri, path_and_query: &str) -> Uri {
    let mut parts = base.clone().into_parts();
    parts.path_and_query = Some(
        path_and_query
            .parse()
            .unwrap_or_else(|_| "/".parse().unwrap()),
    );
    Uri::from_parts(parts).unwrap_or_else(|_| base.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_uri_preserves_query() {
        let base: Uri = "http://127.0.0.1:9000".parse().expect("base");
        let uri = build_uri(&base, "/v1/health?v=1");
        assert_eq!(uri.path(), "/v1/health");
        assert_eq!(uri.query(), Some("v=1"));
    }
}
