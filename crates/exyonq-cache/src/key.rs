//! Plan 12 / WC2 — response cache key materialization.

use std::hash::{Hash, Hasher};

/// Components that form a cache lookup key (no request-time IR).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKeyParts {
    /// Explicit tenant/site scope (0 = unset / Plan 12 microcache placeholders).
    pub site_id: u64,
    /// Backend namespace id ([`crate::CacheNamespace`] as u16; 0 = unset / Plan 12).
    pub namespace: u16,
    /// Canonical runtime-plan `BackendId` index for FPC keys.
    pub backend_id: u32,
    pub runtime_generation: u64,
    pub policy_generation: u64,
    pub route_idx: usize,
    pub method: String,
    pub scheme: String,
    pub host: String,
    pub path: String,
    /// Prefer [`canonicalize_query`] before assigning.
    pub query: String,
    pub content_encoding: String,
}

impl CacheKeyParts {
    pub fn cache_key(&self) -> CacheKey {
        CacheKey(fold_key(self))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey(u64);

impl CacheKey {
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

pub fn build_cache_key(parts: CacheKeyParts) -> CacheKey {
    parts.cache_key()
}

/// Storage key for GET/HEAD — canonical GET representation (v0 tranche 2).
pub fn build_storage_cache_key(mut parts: CacheKeyParts) -> CacheKey {
    parts.method = "GET".into();
    build_cache_key(parts)
}

fn fold_key(parts: &CacheKeyParts) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    hasher.finish()
}

pub fn normalize_host(host: Option<&str>) -> String {
    host.map(|h| {
        let lower = h.trim_end_matches('.').to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix('[') {
            if let Some(end) = rest.find(']') {
                return lower[..=end].to_string();
            }
            return lower;
        }
        lower
            .rsplit_once(':')
            .and_then(|(name, port)| {
                if !name.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
                    Some(name.to_string())
                } else {
                    None
                }
            })
            .unwrap_or(lower)
    })
    .unwrap_or_default()
}

/// Canonical query for cache keys: keep only allowlisted names, sort by name, stable `k=v` join.
///
/// Empty allowlist → empty string (all params dropped from the key).
/// Duplicate names keep first occurrence order among equals after sort-by-name.
pub fn canonicalize_query(raw: &str, allowlist: &[&str]) -> String {
    if raw.is_empty() || allowlist.is_empty() {
        return String::new();
    }
    let allow: std::collections::HashSet<&str> = allowlist.iter().copied().collect();
    let mut kept: Vec<(String, String)> = Vec::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if allow.contains(name) {
            kept.push((name.to_string(), value.to_string()));
        }
    }
    kept.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    kept.into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// Tracking / marketing params that must never enter a cache key even if mistakenly allowlisted.
pub fn is_tracking_query_param(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "gclid" | "fbclid" | "mc_cid" | "mc_eid" | "_ga" | "_gl"
    ) || lower.starts_with("utm_")
}

/// Allowlist filter that also drops known tracking names.
pub fn canonicalize_query_safe(raw: &str, allowlist: &[&str]) -> String {
    let filtered: Vec<&str> = allowlist
        .iter()
        .copied()
        .filter(|n| !is_tracking_query_param(n))
        .collect();
    canonicalize_query(raw, &filtered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(route: usize, host: &str, path: &str, query: &str) -> CacheKeyParts {
        CacheKeyParts {
            site_id: 0,
            namespace: 0,
            backend_id: 0,
            runtime_generation: 1,
            policy_generation: 1,
            route_idx: route,
            method: "GET".into(),
            scheme: "http".into(),
            host: host.into(),
            path: path.into(),
            query: query.into(),
            content_encoding: "identity".into(),
        }
    }

    #[test]
    fn distinct_hosts_do_not_collide() {
        assert_ne!(
            parts(0, "a.test", "/x", "").cache_key(),
            parts(0, "b.test", "/x", "").cache_key()
        );
    }

    #[test]
    fn distinct_query_do_not_collide() {
        assert_ne!(
            parts(0, "h", "/x", "a=1").cache_key(),
            parts(0, "h", "/x", "a=2").cache_key()
        );
    }

    #[test]
    fn distinct_generation_do_not_collide() {
        let a = parts(0, "h", "/", "");
        let mut b = a.clone();
        b.runtime_generation = 2;
        assert_ne!(a.cache_key(), b.cache_key());
    }

    #[test]
    fn head_and_get_share_storage_key() {
        let get = parts(0, "h", "/", "");
        let mut head = get.clone();
        head.method = "HEAD".into();
        assert_eq!(build_storage_cache_key(get), build_storage_cache_key(head));
    }

    #[test]
    fn distinct_site_ids_do_not_collide() {
        let mut a = parts(0, "h", "/x", "");
        let mut b = a.clone();
        a.site_id = 1;
        b.site_id = 2;
        assert_ne!(a.cache_key(), b.cache_key());
    }

    #[test]
    fn distinct_namespaces_do_not_collide() {
        let mut a = parts(0, "h", "/x", "");
        let mut b = a.clone();
        a.namespace = 1; // static
        b.namespace = 3; // fcgi
        assert_ne!(a.cache_key(), b.cache_key());
    }

    #[test]
    fn distinct_backend_ids_do_not_collide() {
        let mut a = parts(0, "h", "/x", "");
        let mut b = a.clone();
        a.backend_id = 1;
        b.backend_id = 2;
        assert_ne!(a.cache_key(), b.cache_key());
    }

    #[test]
    fn empty_allowlist_drops_all_query() {
        assert_eq!(canonicalize_query("a=1&b=2", &[]), "");
        assert_eq!(
            canonicalize_query_safe("utm_source=x&p=1", &["p", "utm_source"]),
            "p=1"
        );
    }

    #[test]
    fn allowlist_sorts_and_filters() {
        assert_eq!(canonicalize_query("z=9&a=1&drop=1", &["z", "a"]), "a=1&z=9");
    }

    #[test]
    fn tracking_params_detected() {
        assert!(is_tracking_query_param("utm_source"));
        assert!(is_tracking_query_param("GCLID"));
        assert!(!is_tracking_query_param("p"));
    }
}
