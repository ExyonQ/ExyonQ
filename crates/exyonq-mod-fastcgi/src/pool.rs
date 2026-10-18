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
//! FastCGI logical pool capacity — PR5-B2 bounded concurrency (not connection pooling).

use std::collections::HashMap;

/// Default in-flight FastCGI requests per pool.
pub const DEFAULT_FCGI_MAX_CONCURRENCY: usize = 16;

/// Maximum configurable in-flight FastCGI requests per pool.
pub const MAX_FCGI_MAX_CONCURRENCY: usize = 4096;

/// Reject `0` or values above [`MAX_FCGI_MAX_CONCURRENCY`].
pub fn validate_max_concurrency(value: u32) -> Option<usize> {
    if (1..=MAX_FCGI_MAX_CONCURRENCY as u32).contains(&value) {
        Some(value as usize)
    } else {
        None
    }
}

/// Resolve per-pool concurrency for registered pools (`pool_id` → capacity).
pub fn resolve_pool_capacities(
    sorted_pool_names: &[String],
    address_by_name: &HashMap<String, String>,
    max_concurrency_by_name: &HashMap<String, u32>,
    env_socket: Option<&str>,
    env_max_concurrency: Option<u32>,
) -> Vec<(u32, usize)> {
    resolve_pool_capacities_with_transport(
        sorted_pool_names,
        address_by_name,
        &HashMap::new(),
        max_concurrency_by_name,
        env_socket,
        env_max_concurrency,
    )
}

/// Resolve capacities for Unix and TCP pools.
pub fn resolve_pool_capacities_with_transport(
    sorted_pool_names: &[String],
    address_by_name: &HashMap<String, String>,
    transport_by_name: &HashMap<String, String>,
    max_concurrency_by_name: &HashMap<String, u32>,
    env_socket: Option<&str>,
    env_max_concurrency: Option<u32>,
) -> Vec<(u32, usize)> {
    let endpoints = crate::adapter::resolve_pool_endpoints(
        sorted_pool_names,
        address_by_name,
        transport_by_name,
        env_socket,
    );
    endpoints
        .into_iter()
        .map(|(pool_id, _)| {
            let name = sorted_pool_names
                .get(pool_id as usize)
                .map(String::as_str)
                .unwrap_or("");
            let configured = max_concurrency_by_name
                .get(name)
                .copied()
                .unwrap_or(DEFAULT_FCGI_MAX_CONCURRENCY as u32);
            let value = if pool_id == 0 {
                env_max_concurrency.unwrap_or(configured)
            } else {
                configured
            };
            let capacity = validate_max_concurrency(value).unwrap_or(DEFAULT_FCGI_MAX_CONCURRENCY);
            (pool_id, capacity)
        })
        .collect()
}

/// Returns true when `address` names a unix domain socket path (re-export for callers).
pub fn is_unix_pool_address(address: &str) -> bool {
    crate::adapter::is_unix_socket_path(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_max_concurrency() {
        assert!(validate_max_concurrency(0).is_none());
    }

    #[test]
    fn accepts_default_and_max() {
        assert_eq!(
            validate_max_concurrency(16).expect("default"),
            DEFAULT_FCGI_MAX_CONCURRENCY
        );
        assert_eq!(
            validate_max_concurrency(4096).expect("max"),
            MAX_FCGI_MAX_CONCURRENCY
        );
    }

    #[test]
    fn rejects_above_max() {
        assert!(validate_max_concurrency(4097).is_none());
    }

    #[test]
    fn per_pool_capacities_are_independent() {
        let names = vec!["main".into(), "admin".into()];
        let mut addresses = HashMap::new();
        addresses.insert("main".into(), "/run/main.sock".into());
        addresses.insert("admin".into(), "/run/admin.sock".into());
        let mut caps = HashMap::new();
        caps.insert("main".into(), 32);
        caps.insert("admin".into(), 4);
        let resolved = resolve_pool_capacities(&names, &addresses, &caps, None, None);
        assert_eq!(resolved, vec![(0, 32), (1, 4)]);
    }

    #[test]
    fn default_capacity_when_unspecified() {
        let names = vec!["php".into()];
        let mut addresses = HashMap::new();
        addresses.insert("php".into(), "/run/php.sock".into());
        let resolved = resolve_pool_capacities(&names, &addresses, &HashMap::new(), None, None);
        assert_eq!(resolved, vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)]);
    }

    #[test]
    fn env_override_applies_to_pool_zero_only() {
        let names = vec!["main".into(), "admin".into()];
        let mut addresses = HashMap::new();
        addresses.insert("main".into(), "/run/main.sock".into());
        addresses.insert("admin".into(), "/run/admin.sock".into());
        let mut caps = HashMap::new();
        caps.insert("main".into(), 32);
        caps.insert("admin".into(), 4);
        let resolved = resolve_pool_capacities(&names, &addresses, &caps, None, Some(1));
        assert_eq!(resolved, vec![(0, 1), (1, 4)]);
    }
}
