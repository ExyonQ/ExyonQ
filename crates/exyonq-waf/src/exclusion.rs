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

use exyonq_waf_api::{WafRequest, WafRuleId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompiledExclusion {
    pub host: Option<String>,
    pub path_prefix: Option<String>,
    pub method: Option<String>,
    pub rule_ids: Vec<String>,
}

impl CompiledExclusion {
    /// True when this exclusion applies to `(req, rule_id)`.
    ///
    /// All *present* fields must match. Empty `rule_ids` matches **nothing**
    /// (not any rule). Path prefix matches only exact path or `prefix + "/" + …`.
    pub(crate) fn matches(&self, req: &WafRequest<'_>, rule_id: &WafRuleId) -> bool {
        if let Some(host) = self.host.as_deref() {
            match req.host {
                Some(h) if host_eq(h, host) => {}
                _ => return false,
            }
        }
        if let Some(prefix) = self.path_prefix.as_deref() {
            if !path_matches_prefix(req.path, prefix) {
                return false;
            }
        }
        if let Some(method) = self.method.as_deref() {
            if !req.method.eq_ignore_ascii_case(method) {
                return false;
            }
        }
        // Empty rule_ids → match nothing (explicit allowlist of rule ids required).
        if self.rule_ids.is_empty() {
            return false;
        }
        self.rule_ids
            .iter()
            .any(|id| id.as_str() == rule_id.as_str())
    }
}

fn host_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Prefix match that does not treat `/api` as a prefix of `/api2`.
fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    if path == prefix {
        return true;
    }
    let mut pfx = prefix.to_string();
    if !pfx.ends_with('/') {
        pfx.push('/');
    }
    path.starts_with(&pfx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_waf_api::EmptyHeaders;
    use std::net::{IpAddr, Ipv4Addr};

    fn req<'a>(
        method: &'a str,
        host: Option<&'a str>,
        path: &'a str,
        headers: &'a EmptyHeaders,
    ) -> WafRequest<'a> {
        WafRequest {
            method,
            host,
            path,
            query: None,
            headers,
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            route_id: None,
        }
    }

    #[test]
    fn exclusion_requires_all_fields() {
        let headers = EmptyHeaders;
        let r = req("POST", Some("example.com"), "/upload/x", &headers);
        let ex = CompiledExclusion {
            host: Some("example.com".into()),
            path_prefix: Some("/upload".into()),
            method: Some("POST".into()),
            rule_ids: vec!["EXY-XSS-1001".into()],
        };
        assert!(ex.matches(&r, &WafRuleId::new("EXY-XSS-1001")));
        assert!(!ex.matches(&r, &WafRuleId::new("EXY-SQL-1")));
        assert!(!ex.matches(
            &req("GET", Some("example.com"), "/upload/x", &headers),
            &WafRuleId::new("EXY-XSS-1001")
        ));
    }

    #[test]
    fn empty_rule_ids_matches_nothing() {
        let headers = EmptyHeaders;
        let r = req("GET", Some("example.com"), "/api/x", &headers);
        let ex = CompiledExclusion {
            host: Some("example.com".into()),
            path_prefix: Some("/api".into()),
            method: None,
            rule_ids: Vec::new(),
        };
        assert!(!ex.matches(&r, &WafRuleId::new("EXY-XSS-1001")));
    }

    #[test]
    fn path_prefix_does_not_match_sibling_prefix() {
        let headers = EmptyHeaders;
        let ex = CompiledExclusion {
            host: None,
            path_prefix: Some("/api".into()),
            method: None,
            rule_ids: vec!["EXY-XSS-1001".into()],
        };
        assert!(ex.matches(
            &req("GET", None, "/api", &headers),
            &WafRuleId::new("EXY-XSS-1001")
        ));
        assert!(ex.matches(
            &req("GET", None, "/api/v1", &headers),
            &WafRuleId::new("EXY-XSS-1001")
        ));
        assert!(!ex.matches(
            &req("GET", None, "/api2", &headers),
            &WafRuleId::new("EXY-XSS-1001")
        ));
    }
}
